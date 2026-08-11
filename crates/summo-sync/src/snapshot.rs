//! What the vault looks like right now, and what it looked like at the last sync.
//!
//! A snapshot is a map from vault-relative path to a content hash. That is the whole of it — no
//! timestamps in the comparison, no sequence numbers, no version vectors. Content is the identity:
//! a file whose bytes have not changed has not changed, whatever its mtime says after a restore
//! from backup, a `git checkout`, or a copy between machines that disagree about clocks.
//!
//! ## Why mtime is recorded but not compared
//!
//! [`Entry::modified`] is kept because it is what tells a *human* which of two versions is newer
//! when they have to choose, and because rehashing an unchanged 200 MB vault on every sync would be
//! wasteful. It is a cache key, not evidence. Two machines' clocks disagree, some filesystems store
//! seconds and others nanoseconds, and "newest wins" silently destroys the older edit — which is
//! the failure people actually report from file sync tools.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result};

/// One file in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// BLAKE3 of the contents, hex. The file's identity.
    pub hash: String,
    pub size: u64,
    /// Seconds since the epoch. For showing a person which side is newer, never for deciding.
    #[serde(default)]
    pub modified: u64,
}

/// The vault, as a map of path to content.
///
/// `BTreeMap` rather than a hash map so a serialised snapshot is byte-identical between runs.
/// A diff of two snapshot files should show what changed in the vault, not what changed in the
/// iteration order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub files: BTreeMap<String, Entry>,
}

impl Snapshot {
    /// Walk a directory and hash everything worth syncing.
    pub fn scan(root: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        if !root.exists() {
            return Ok(Self { files });
        }

        for entry in walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_ignored(e.file_name().to_string_lossy().as_ref()))
        {
            let entry = entry.map_err(|e| Error::Other(format!("cannot walk the vault: {e}")))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Some(key) = relative(root, path) else {
                continue;
            };

            let meta = entry
                .metadata()
                .map_err(|e| Error::Other(format!("cannot read {}: {e}", path.display())))?;
            let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;

            files.insert(
                key,
                Entry {
                    hash: blake3::hash(&bytes).to_hex().to_string(),
                    size: meta.len(),
                    modified: seconds(&meta),
                },
            );
        }
        Ok(Self { files })
    }

    /// Read the snapshot recorded at the end of the last sync.
    ///
    /// A missing or unreadable file is an empty snapshot, which makes the next sync behave like a
    /// first one: everything looks new, nothing is deleted. That is the safe direction to fail in —
    /// treating an unreadable base as "everything was deleted" would propagate the deletions.
    #[must_use]
    pub fn read(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Other(format!("cannot serialise the snapshot: {e}")))?;
        std::fs::write(path, text).map_err(|e| Error::io(path, e))
    }

    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Entry> {
        self.files.get(path)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Every path in either snapshot, in order and without repeats.
    #[must_use]
    pub fn paths_union(&self, other: &Self) -> Vec<String> {
        let mut all: Vec<String> = self
            .files
            .keys()
            .chain(other.files.keys())
            .cloned()
            .collect();
        all.sort();
        all.dedup();
        all
    }
}

/// Whether a name should never be synced.
///
/// Derived data and machine-local state. `index.db` is rebuilt from the vault by design (ADR 0002),
/// so syncing it would move bytes to reconstruct something each machine can produce itself — and
/// would sync a SQLite file, which is the single worst thing to merge.
fn is_ignored(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".DS_Store" | "index.db" | ".summo-sync" | "node_modules"
    ) || name.ends_with(".tmp")
        // Conflict copies stay on the machine that made them. Syncing them would hand every other
        // machine a conflict it was not party to, and they would multiply.
        || name.contains(".conflict-")
}

/// Path relative to the root, with forward slashes on every platform.
///
/// The separator matters: a vault synced from Windows and read on Linux must agree on what a path
/// *is*, or every file looks new on the other side and the first sync duplicates the whole vault.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let text = rest.to_string_lossy().replace('\\', "/");
    (!text.is_empty()).then_some(text)
}

fn seconds(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The absolute path of a snapshot entry.
#[must_use]
pub fn absolute(root: &Path, relative: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for part in relative.split('/') {
        out.push(part);
    }
    out
}

/// Whether a relative path is safe to write under the root.
///
/// A path arrives from the *remote*, which in the paid tier is a server, and in a self-hosted setup
/// is whatever the user pointed at. `../../.ssh/authorized_keys` in a manifest must not become a
/// write outside the vault. Checked on the string rather than by canonicalising, because the file
/// does not exist yet and `canonicalize` cannot tell you where it would land.
#[must_use]
pub fn is_safe(relative: &str) -> bool {
    !relative.is_empty()
        && !relative.starts_with('/')
        && !relative.contains('\\')
        && !relative.contains("//")
        && !relative
            .split('/')
            .any(|part| part.is_empty() || part == ".." || part == "." || part.contains(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for (name, body) in files {
            let path = tmp.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        tmp
    }

    #[test]
    fn a_scan_finds_every_file_including_ones_in_folders() {
        let dir = vault(&[("a.md", "one"), ("Product/b.md", "two")]);
        let snap = Snapshot::scan(dir.path()).unwrap();

        assert_eq!(snap.len(), 2);
        assert!(snap.get("a.md").is_some());
        assert!(
            snap.get("Product/b.md").is_some(),
            "{:?}",
            snap.files.keys()
        );
    }

    /// Content is the identity. A file restored from a backup has a new mtime and the same bytes,
    /// and must not look changed.
    #[test]
    fn the_hash_follows_the_bytes_and_not_the_timestamp() {
        let dir = vault(&[("a.md", "one")]);
        let first = Snapshot::scan(dir.path()).unwrap();

        let path = dir.path().join("a.md");
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();

        let again = Snapshot::scan(dir.path()).unwrap();
        assert_eq!(
            first.get("a.md").unwrap().hash,
            again.get("a.md").unwrap().hash
        );
    }

    #[test]
    fn different_contents_hash_differently() {
        let dir = vault(&[("a.md", "one"), ("b.md", "two")]);
        let snap = Snapshot::scan(dir.path()).unwrap();
        assert_ne!(
            snap.get("a.md").unwrap().hash,
            snap.get("b.md").unwrap().hash
        );
    }

    /// Derived data is rebuilt from the vault by design, so syncing it moves bytes to reconstruct
    /// what each machine can make itself — and `index.db` is a SQLite file, the worst thing to
    /// merge.
    #[test]
    fn derived_and_machine_local_files_are_not_synced() {
        let dir = vault(&[
            ("a.md", "one"),
            ("index.db", "sqlite"),
            (".DS_Store", "junk"),
            ("draft.tmp", "half"),
        ]);
        let snap = Snapshot::scan(dir.path()).unwrap();
        assert_eq!(snap.len(), 1, "{:?}", snap.files.keys());
    }

    /// Conflict copies would otherwise hand every other machine a conflict it was not party to,
    /// and then multiply.
    #[test]
    fn a_conflict_copy_stays_on_the_machine_that_made_it() {
        let dir = vault(&[("a.md", "one"), ("a.conflict-laptop.md", "mine")]);
        assert_eq!(Snapshot::scan(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn a_missing_directory_is_an_empty_snapshot() {
        let snap = Snapshot::scan(Path::new("/nonexistent/vault")).unwrap();
        assert!(snap.is_empty());
    }

    /// Treating an unreadable base as "everything was deleted" would propagate the deletions to
    /// every other machine. Empty means "sync as if for the first time", which loses nothing.
    #[test]
    fn an_unreadable_base_is_empty_rather_than_a_mass_deletion() {
        let dir = vault(&[("base.json", "{ not json")]);
        assert!(Snapshot::read(&dir.path().join("base.json")).is_empty());
        assert!(Snapshot::read(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn a_snapshot_survives_a_round_trip() {
        let dir = vault(&[("a.md", "one"), ("Product/b.md", "two")]);
        let snap = Snapshot::scan(dir.path()).unwrap();

        let path = dir.path().join("state").join("base.json");
        snap.write(&path).unwrap();
        assert_eq!(Snapshot::read(&path), snap);
    }

    /// A diff of two snapshot files should show what changed in the vault, not what changed in the
    /// iteration order.
    #[test]
    fn serialisation_is_stable_between_runs() {
        let dir = vault(&[("b.md", "two"), ("a.md", "one"), ("c.md", "three")]);
        let first = serde_json::to_string(&Snapshot::scan(dir.path()).unwrap()).unwrap();
        let again = serde_json::to_string(&Snapshot::scan(dir.path()).unwrap()).unwrap();
        assert_eq!(first, again);
    }

    #[test]
    fn the_union_of_two_snapshots_has_no_repeats() {
        let a = vault(&[("a.md", "1"), ("shared.md", "1")]);
        let b = vault(&[("b.md", "1"), ("shared.md", "2")]);
        let paths = Snapshot::scan(a.path())
            .unwrap()
            .paths_union(&Snapshot::scan(b.path()).unwrap());
        assert_eq!(paths, vec!["a.md", "b.md", "shared.md"]);
    }

    // ---- paths from the other side ---------------------------------------------------------

    /// These arrive from a server. A traversal in a manifest must not become a write outside the
    /// vault.
    #[test]
    fn a_path_that_escapes_the_vault_is_refused() {
        assert!(!is_safe("../secrets.md"));
        assert!(!is_safe("notes/../../.ssh/authorized_keys"));
        assert!(!is_safe("/etc/passwd"));
        assert!(!is_safe(".."));
        assert!(!is_safe("a//b.md"));
        assert!(!is_safe(""));
        // Windows drive letters and separators, which a Linux `split('/')` would wave through.
        assert!(!is_safe("C:/Windows/system32"));
        assert!(!is_safe("notes\\..\\..\\x"));
    }

    #[test]
    fn an_ordinary_path_is_allowed() {
        assert!(is_safe("meetings/2026-08-10-sync.md"));
        assert!(is_safe("agents/scribe/MEMORY.md"));
        assert!(is_safe("a.md"));
    }

    #[test]
    fn a_relative_path_resolves_under_the_root() {
        let resolved = absolute(Path::new("/vault"), "meetings/a.md");
        assert!(resolved.ends_with("meetings/a.md"));
        assert!(resolved.starts_with("/vault"));
    }
}
