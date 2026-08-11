//! One sync, start to finish.
//!
//! ```text
//!   scan local ─┐
//!               ├─► plan ─► for each step: merge / seal / write ─► publish manifest ─► save base
//!   open remote ┘
//! ```
//!
//! ## Order matters, twice
//!
//! **Blobs before the manifest.** The manifest is what the other machine reads to know what exists.
//! Publishing it before the blobs it names means a run interrupted in the middle leaves the other
//! machine pointing at files it cannot fetch. The other way round leaves unreferenced blobs, which
//! cost disk and nothing else.
//!
//! **The base last.** [`crate::snapshot::Snapshot`] of what the two sides now agree on is written
//! only after everything else succeeded. If the run dies halfway, the base still describes the last
//! *complete* sync, so the next run re-examines the work in between rather than assuming it landed.
//! An optimistically-written base is how a sync tool loses a file: it records agreement that never
//! happened, and the next run sees "unchanged since the base" and does nothing.
//!
//! ## Nothing is deleted locally without evidence
//!
//! A local delete happens only when the base proves the file was unchanged here since the last
//! sync, and the remote no longer has it. Every other shape either uploads, downloads, merges, or
//! leaves a conflict copy.

use std::path::Path;

use summo_core::{Error, Result};

use crate::crypto::{Key, Sealed};
use crate::plan::{Action, Side};
use crate::remote::{Manifest, Remote};
use crate::snapshot::{self, Snapshot};

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub summary: crate::plan::Summary,
    /// Files that could not be reconciled, with the copy written beside each.
    pub conflicts: Vec<Conflict>,
    /// Paths the remote offered that were not safe to write.
    pub refused: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    /// Where the other machine's version was written.
    pub copy: String,
}

impl Outcome {
    #[must_use]
    pub fn is_quiet(&self) -> bool {
        self.conflicts.is_empty() && self.refused.is_empty()
    }
}

/// Run one sync between a vault and a remote.
///
/// `machine` names this machine and ends up in the name of any conflict copy the *other* side
/// causes, so a user opening their vault can tell whose version they are looking at.
pub fn sync(
    vault: &Path,
    state: &Path,
    remote: &mut dyn Remote,
    key: &Key,
    machine: &str,
) -> Result<Outcome> {
    let local = Snapshot::scan(vault)?;
    let base = Snapshot::read(&state.join("base.json"));

    let remote_manifest = match remote.manifest()? {
        Some(bytes) => {
            let sealed = Sealed::from_bytes(&bytes)?;
            // The manifest is bound to a fixed name rather than to a path, since it is the one blob
            // that is not a file in the vault.
            let plain = key.open(MANIFEST_AAD, &sealed)?;
            serde_json::from_slice::<Manifest>(&plain)
                .map_err(|e| Error::Other(format!("the remote manifest is malformed: {e}")))?
        }
        None => Manifest::default(),
    };
    let their_snapshot = remote_manifest.to_snapshot();

    let plan = crate::plan::plan(&local, &their_snapshot, &base);
    let mut outcome = Outcome {
        summary: plan.summary(),
        conflicts: Vec::new(),
        refused: plan.refused.clone(),
    };

    // What the two sides agree on once this run finishes. Built as we go and written only at the
    // end, so a crash leaves the last complete sync recorded rather than a half-finished one.
    let mut agreed = base.clone();

    for step in &plan.steps {
        let path = &step.path;
        let local_file = snapshot::absolute(vault, path);

        match &step.action {
            Action::Upload | Action::Resurrect { edited_on: Side::Local } => {
                let bytes = std::fs::read(&local_file).map_err(|e| Error::io(&local_file, e))?;
                push(remote, key, &bytes)?;
                agreed.files.insert(path.clone(), entry_for(&bytes, &local_file));
            }

            Action::Download | Action::Resurrect { edited_on: Side::Remote } => {
                let want = their_snapshot.get(path).map(|e| e.hash.as_str()).unwrap_or_default();
                let bytes = fetch(remote, key, want, path)?;
                write_file(&local_file, &bytes)?;
                agreed.files.insert(path.clone(), entry_for(&bytes, &local_file));
            }

            Action::DeleteRemote => {
                // The manifest is what says a file exists; the blob is content-addressed and may
                // still be some other version's ancestor. Unreferenced blobs are collectable, which
                // is a separate job from deciding a file is gone.
                agreed.files.remove(path);
            }

            Action::DeleteLocal => {
                // Removing the file is the whole action; a missing one is already the desired
                // state, which happens when a previous run died between here and the base write.
                if let Err(e) = std::fs::remove_file(&local_file)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(Error::io(&local_file, e));
                }
                agreed.files.remove(path);
            }

            Action::Merge => {
                let mine = std::fs::read(&local_file).map_err(|e| Error::io(&local_file, e))?;
                let want = their_snapshot.get(path).map(|e| e.hash.as_str()).unwrap_or_default();
                let theirs = fetch(remote, key, want, path)?;

                // The ancestor's *contents*. Blobs are addressed by content, so the version the two
                // sides last agreed on is still there under its own hash even though both have
                // since moved past it — which is the entire reason for addressing them that way.
                // When it cannot be had, an empty ancestor makes any difference a conflict: the
                // safe direction, since a conflict copies rather than merges and copies lose
                // nothing.
                let ancestor = ancestor_bytes(remote, key, path, base.get(path));

                match (
                    String::from_utf8(mine.clone()),
                    String::from_utf8(theirs.clone()),
                    String::from_utf8(ancestor.clone()),
                ) {
                    (Ok(mine_text), Ok(their_text), Ok(base_text)) => {
                        match crate::merge::merge(&base_text, &mine_text, &their_text) {
                            crate::merge::Merged::Clean(text) => {
                                write_file(&local_file, text.as_bytes())?;
                                push(remote, key, text.as_bytes())?;
                                agreed
                                    .files
                                    .insert(path.clone(), entry_for(text.as_bytes(), &local_file));
                            }
                            crate::merge::Merged::Conflict { .. } => {
                                outcome
                                    .conflicts
                                    .push(keep_both(vault, path, &theirs, remote.name().as_str())?);
                                // Deliberately not recorded as agreed: nothing was reconciled, so
                                // the next run must look at this file again.
                            }
                        }
                    }
                    // Not text. A merge is meaningless, so both versions are kept — the same answer
                    // as a conflict, reached for a different reason.
                    _ => {
                        outcome
                            .conflicts
                            .push(keep_both(vault, path, &theirs, remote.name().as_str())?);
                    }
                }
            }
        }
    }

    // The manifest describes what the remote holds *now*, which is the base plus whatever this run
    // could not settle — those files keep whatever the remote already had.
    let mut published = agreed.clone();
    for conflict in &outcome.conflicts {
        if let Some(theirs) = their_snapshot.get(&conflict.path) {
            published.files.insert(conflict.path.clone(), theirs.clone());
        }
    }

    let manifest = serde_json::to_vec(&Manifest::from_snapshot(&published))
        .map_err(|e| Error::Other(format!("cannot serialise the manifest: {e}")))?;
    // Last, after every blob it names.
    remote.put_manifest(&key.seal(MANIFEST_AAD, &manifest)?.to_bytes())?;

    published.write(&state.join("base.json"))?;
    tracing::info!(machine, summary = %outcome.summary, "sync finished");
    Ok(outcome)
}

/// The name the manifest is bound to. Not a vault path, so it cannot collide with one.
const MANIFEST_AAD: &str = "\u{0}summo-manifest";

/// Store one version, under the hash of its own contents.
///
/// Content-addressed for the same reason the model store is: the same bytes are stored once
/// however many paths hold them, and — the part sync depends on — a version stays reachable after
/// both sides have edited past it, so it can serve as the ancestor of a later merge.
///
/// The ciphertext is bound to the content hash rather than to a path. That keeps the guarantee the
/// path binding gave — a relay cannot serve one blob's bytes under another blob's id — while
/// letting a blob belong to more than one path.
fn push(remote: &mut dyn Remote, key: &Key, bytes: &[u8]) -> Result<String> {
    let hash = blake3::hash(bytes).to_hex().to_string();
    let id = key.id_for(&hash);
    // Already there means already correct: the id is derived from the contents.
    if remote.get(&id)?.is_none() {
        remote.put(&id, &key.seal(&hash, bytes)?.to_bytes())?;
    }
    Ok(hash)
}

/// Fetch the version with a given content hash.
fn fetch(remote: &dyn Remote, key: &Key, hash: &str, path: &str) -> Result<Vec<u8>> {
    let bytes = remote.get(&key.id_for(hash))?.ok_or_else(|| {
        Error::msg(
            "sync.missing_blob",
            format!("the remote lists {path} but does not have its contents"),
        )
    })?;
    let plain = key.open(hash, &Sealed::from_bytes(&bytes)?)?;
    // The id, the AAD and the contents all have to agree. They cannot disagree without the relay
    // having been tampered with, and this is cheap.
    if blake3::hash(&plain).to_hex().to_string() != hash {
        return Err(Error::msg(
            "sync.wrong_contents",
            format!("the contents stored for {path} are not what was asked for"),
        ));
    }
    Ok(plain)
}

/// The common ancestor's bytes, if they can still be had.
///
/// Empty when they cannot, which makes every difference a conflict rather than a guess.
fn ancestor_bytes(
    remote: &dyn Remote,
    key: &Key,
    path: &str,
    base: Option<&crate::snapshot::Entry>,
) -> Vec<u8> {
    let Some(base) = base else {
        return Vec::new();
    };
    fetch(remote, key, &base.hash, path).unwrap_or_default()
}

/// Write the other side's version beside the local one, leaving the local one untouched.
fn keep_both(vault: &Path, path: &str, theirs: &[u8], machine: &str) -> Result<Conflict> {
    let copy = crate::merge::conflict_name(path, machine);
    write_file(&snapshot::absolute(vault, &copy), theirs)?;
    tracing::warn!(path, copy, "could not merge; kept both versions");
    Ok(Conflict {
        path: path.to_string(),
        copy,
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::write(path, bytes).map_err(|e| Error::io(path, e))
}

fn entry_for(bytes: &[u8], path: &Path) -> crate::snapshot::Entry {
    crate::snapshot::Entry {
        hash: blake3::hash(bytes).to_hex().to_string(),
        size: bytes.len() as u64,
        modified: std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::Directory;

    /// A machine: its vault, its sync state, and the passphrase-derived key.
    struct Machine {
        dir: tempfile::TempDir,
        key: Key,
        name: String,
    }

    impl Machine {
        fn new(name: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("vault")).unwrap();
            std::fs::create_dir_all(dir.path().join("state")).unwrap();
            Self {
                dir,
                // A fixed salt so both machines derive the same key, which is what a shared
                // passphrase means.
                key: Key::derive("a shared passphrase", b"0123456789abcdef").unwrap(),
                name: name.to_string(),
            }
        }

        fn vault(&self) -> PathBuf {
            self.dir.path().join("vault")
        }

        fn state(&self) -> PathBuf {
            self.dir.path().join("state")
        }

        fn write(&self, path: &str, body: &str) {
            let full = snapshot::absolute(&self.vault(), path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }

        fn read(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(snapshot::absolute(&self.vault(), path)).ok()
        }

        fn remove(&self, path: &str) {
            std::fs::remove_file(snapshot::absolute(&self.vault(), path)).unwrap();
        }

        fn sync(&self, remote: &mut dyn Remote) -> Outcome {
            sync(&self.vault(), &self.state(), remote, &self.key, &self.name).unwrap()
        }

        fn files(&self) -> Vec<String> {
            let mut out: Vec<String> = Snapshot::scan(&self.vault())
                .unwrap()
                .files
                .into_keys()
                .collect();
            out.sort();
            out
        }
    }

    use std::path::PathBuf;

    fn relay() -> (tempfile::TempDir, Directory) {
        let tmp = tempfile::tempdir().unwrap();
        let remote = Directory::open(tmp.path()).unwrap();
        (tmp, remote)
    }

    #[test]
    fn a_first_sync_uploads_everything() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        laptop.write("meetings/a.md", "# Họp\n");
        laptop.write("notes/b.md", "# Ghi chú\n");

        let outcome = laptop.sync(&mut remote);
        assert_eq!(outcome.summary.uploaded, 2, "{outcome:?}");
        assert!(outcome.is_quiet());
    }

    /// The whole point, in one test.
    #[test]
    fn a_file_written_on_one_machine_arrives_on_the_other() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("meetings/a.md", "# Họp\n\nnội dung\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        assert_eq!(desktop.read("meetings/a.md").as_deref(), Some("# Họp\n\nnội dung\n"));
    }

    #[test]
    fn syncing_twice_with_no_changes_does_nothing() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        laptop.write("a.md", "one\n");
        laptop.sync(&mut remote);

        let again = laptop.sync(&mut remote);
        assert_eq!(again.summary, crate::plan::Summary::default(), "{again:?}");
    }

    #[test]
    fn an_edit_on_one_machine_reaches_the_other() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("a.md", "one\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        laptop.write("a.md", "one, edited\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        assert_eq!(desktop.read("a.md").as_deref(), Some("one, edited\n"));
    }

    /// Without a base this is indistinguishable from "they created it", and the deleted file comes
    /// back on every sync for ever.
    #[test]
    fn a_deletion_propagates_instead_of_the_file_coming_back() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("a.md", "one\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);
        assert_eq!(desktop.files(), vec!["a.md"]);

        laptop.remove("a.md");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);
        assert!(desktop.files().is_empty(), "{:?}", desktop.files());

        // And it stays gone rather than reappearing on the next run.
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);
        assert!(desktop.files().is_empty());
    }

    /// The case a three-way merge exists for: two people edit the same meeting in different places.
    #[test]
    fn edits_to_different_parts_of_one_file_both_survive() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("a.md", "# Họp\n\nmở đầu\n\nkết luận\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        laptop.write("a.md", "# Họp\n\nMỞ ĐẦU\n\nkết luận\n");
        desktop.write("a.md", "# Họp\n\nmở đầu\n\nKẾT LUẬN\n");

        laptop.sync(&mut remote);
        let outcome = desktop.sync(&mut remote);

        let merged = desktop.read("a.md").unwrap();
        assert!(merged.contains("MỞ ĐẦU"), "{merged}");
        assert!(merged.contains("KẾT LUẬN"), "{merged}");
        assert!(outcome.conflicts.is_empty(), "{outcome:?}");
    }

    /// Two machines each adding an action item — the most likely real conflict in this product.
    #[test]
    fn two_machines_adding_different_tasks_keep_both() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("a.md", "## Việc cần làm\n\n- [ ] @ngoc Chốt spec\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        laptop.write("a.md", "## Việc cần làm\n\n- [ ] @ngoc Chốt spec\n- [ ] @minh Đo M1\n");
        desktop.write("a.md", "## Việc cần làm\n\n- [ ] @ngoc Chốt spec\n- [ ] @viet Release note\n");

        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        let merged = desktop.read("a.md").unwrap();
        assert!(merged.contains("Đo M1"), "{merged}");
        assert!(merged.contains("Release note"), "{merged}");
    }

    /// Markers in a meeting note would render as garbage in Obsidian. Two whole files, both
    /// openable, and the user decides.
    #[test]
    fn an_unmergeable_conflict_keeps_both_files_and_touches_neither() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("a.md", "một dòng\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        laptop.write("a.md", "bản của laptop\n");
        desktop.write("a.md", "bản của desktop\n");

        laptop.sync(&mut remote);
        let outcome = desktop.sync(&mut remote);

        assert_eq!(outcome.conflicts.len(), 1, "{outcome:?}");
        assert_eq!(
            desktop.read("a.md").as_deref(),
            Some("bản của desktop\n"),
            "the local file must be left exactly as it was"
        );

        let copy = &outcome.conflicts[0].copy;
        assert_eq!(desktop.read(copy).as_deref(), Some("bản của laptop\n"));
        assert!(!desktop.read(copy).unwrap().contains("<<<<"));
    }

    /// A conflict copy on one machine must not become a conflict on every other machine.
    #[test]
    fn a_conflict_copy_does_not_spread() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("a.md", "một\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        laptop.write("a.md", "laptop\n");
        desktop.write("a.md", "desktop\n");
        laptop.sync(&mut remote);
        let outcome_of = desktop.sync(&mut remote);

        // The copy exists here — read from disk rather than from a scan, which deliberately
        // ignores conflict copies.
        let copy = &outcome_of.conflicts[0].copy;
        assert!(desktop.read(copy).is_some(), "the copy should be on disk");
        // …and never reaches the other machine.
        desktop.sync(&mut remote);
        laptop.sync(&mut remote);
        assert_eq!(laptop.files(), vec!["a.md"], "{:?}", laptop.files());
    }

    /// An edit is worth more than a deletion: restoring costs one repeated deletion, deleting costs
    /// work somebody may not miss for weeks.
    #[test]
    fn a_file_edited_here_and_deleted_there_comes_back() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("a.md", "một\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        desktop.remove("a.md");
        desktop.sync(&mut remote);

        laptop.write("a.md", "một, đã sửa\n");
        let outcome = laptop.sync(&mut remote);

        assert_eq!(outcome.summary.resurrected, 1, "{outcome:?}");
        assert_eq!(laptop.read("a.md").as_deref(), Some("một, đã sửa\n"));
    }

    // ---- what the relay can see ---------------------------------------------------------------

    /// If the relay can read this, everything else in the crate is theatre.
    #[test]
    fn the_relay_holds_no_readable_names_or_contents() {
        let (relay_dir, mut remote) = relay();
        let laptop = Machine::new("laptop");
        laptop.write(
            "meetings/2026-08-10-thuong-vu-sap-nhap.md",
            "# Thương vụ\n\ngiá 40 triệu\n",
        );
        laptop.sync(&mut remote);

        let mut seen = Vec::new();
        for entry in walkdir::WalkDir::new(relay_dir.path()).into_iter().flatten() {
            seen.push(entry.file_name().to_string_lossy().into_owned());
            if entry.file_type().is_file() {
                let bytes = std::fs::read(entry.path()).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                assert!(!text.contains("40 triệu"), "contents readable in {:?}", entry.path());
                assert!(!text.contains("Thương vụ"), "contents readable in {:?}", entry.path());
                assert!(!text.contains("meetings"), "a path leaked in {:?}", entry.path());
            }
        }
        let names = seen.join(" ");
        assert!(!names.contains("sap-nhap"), "a filename leaked: {names}");
    }

    /// Another vault's passphrase must not open this one.
    #[test]
    fn a_different_passphrase_cannot_read_the_relay() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        laptop.write("a.md", "bí mật\n");
        laptop.sync(&mut remote);

        let intruder = Machine::new("intruder");
        let wrong = Key::derive("a different passphrase", b"0123456789abcdef").unwrap();
        let result = sync(
            &intruder.vault(),
            &intruder.state(),
            &mut remote,
            &wrong,
            "intruder",
        );
        assert!(result.is_err(), "the vault opened with the wrong passphrase");
    }

    // ---- crash safety --------------------------------------------------------------------------

    /// An optimistically-written base is how a sync tool loses a file: it records agreement that
    /// never happened, and the next run sees "unchanged" and does nothing.
    #[test]
    fn a_run_that_fails_does_not_record_agreement() {
        struct Breaks(Directory);
        impl Remote for Breaks {
            fn salt(&self) -> Result<Option<Vec<u8>>> {
                self.0.salt()
            }
            fn put_salt(&mut self, salt: &[u8]) -> Result<()> {
                self.0.put_salt(salt)
            }
            fn manifest(&self) -> Result<Option<Vec<u8>>> {
                self.0.manifest()
            }
            fn put_manifest(&mut self, _sealed: &[u8]) -> Result<()> {
                Err(Error::Other("the network went away".into()))
            }
            fn get(&self, id: &str) -> Result<Option<Vec<u8>>> {
                self.0.get(id)
            }
            fn put(&mut self, id: &str, sealed: &[u8]) -> Result<()> {
                self.0.put(id, sealed)
            }
            fn delete(&mut self, id: &str) -> Result<()> {
                self.0.delete(id)
            }
        }

        let (_r, directory) = relay();
        let mut remote = Breaks(directory);
        let laptop = Machine::new("laptop");
        laptop.write("a.md", "một\n");

        assert!(
            sync(&laptop.vault(), &laptop.state(), &mut remote, &laptop.key, "laptop").is_err()
        );
        assert!(
            Snapshot::read(&laptop.state().join("base.json")).is_empty(),
            "the base must still describe the last complete sync"
        );
    }

    /// A vault with folders has to arrive with its folders.
    #[test]
    fn a_file_in_a_folder_arrives_in_that_folder() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("meetings/Sản phẩm/a.md", "# Họp\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        assert_eq!(desktop.files(), vec!["meetings/Sản phẩm/a.md"]);
    }

    /// Agents are directories in the vault, so they ride the same sync as everything else. This is
    /// the whole reason they were built as files.
    #[test]
    fn an_agent_and_its_memory_sync_like_any_other_file() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("agents/AGENTS.md", "# Rules\n");
        laptop.write("agents/scribe/AGENT.md", "---\nname: Scribe\n---\n\nWrite.\n");
        laptop.write("agents/scribe/MEMORY.md", "- 2026-08-11 — Ngọc leads product\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        assert!(desktop.read("agents/scribe/AGENT.md").unwrap().contains("Scribe"));
        assert!(desktop.read("agents/scribe/MEMORY.md").unwrap().contains("Ngọc"));
    }

    /// Two machines whose agents each learned something different should end up knowing both.
    #[test]
    fn two_agents_remembering_different_things_end_up_knowing_both() {
        let (_r, mut remote) = relay();
        let laptop = Machine::new("laptop");
        let desktop = Machine::new("desktop");

        laptop.write("agents/scribe/MEMORY.md", "# Memory\n\n- 2026-08-10 — a\n");
        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        laptop.write("agents/scribe/MEMORY.md", "# Memory\n\n- 2026-08-10 — a\n- 2026-08-11 — laptop learned b\n");
        desktop.write("agents/scribe/MEMORY.md", "# Memory\n\n- 2026-08-10 — a\n- 2026-08-11 — desktop learned c\n");

        laptop.sync(&mut remote);
        desktop.sync(&mut remote);

        let memory = desktop.read("agents/scribe/MEMORY.md").unwrap();
        assert!(memory.contains("laptop learned b"), "{memory}");
        assert!(memory.contains("desktop learned c"), "{memory}");
    }
}
