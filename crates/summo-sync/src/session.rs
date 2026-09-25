//! One sync, against a folder, for whoever asked.
//!
//! The rest of this crate is the *how*: scan, plan, three-way merge, seal, push. This is the part
//! every caller needs and none of them should write twice — where the salt comes from, where the
//! base snapshot lives, what a run returns, and which folders are refused before a passphrase is
//! asked for.
//!
//! It exists because there was one caller and it was a subcommand. Ninety-four tests' worth of
//! working sync, a product page advertising "encrypted sync between your machines through any
//! shared folder", and from inside the app there was no folder to choose, no button to press and
//! no way to find out it was there. The glue lives here rather than in the daemon so the two are
//! doors into one implementation — and so a build of the command line with no daemon in it still
//! has `summo sync`.
//!
//! ## The passphrase
//!
//! It arrives in a request body and is dropped when the run ends. Nothing writes it down.
//!
//! The command line refuses to take it as an argument, and that refusal is not arbitrary: an
//! argument is visible in `ps` to every other user on the machine and lands in the shell history
//! file. A POST body over the loopback socket is in neither — it is readable by anything that
//! already holds this daemon's bearer token, which is the same thing that can already read every
//! meeting in the vault. So the body is not a step down from the prompt; storing it would be.
//!
//! `Settings::sync` therefore holds a folder and a machine name and no key material. See the note
//! on that struct.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result, paths::Paths};

use crate::{Action, Key, Manifest, Remote, Sealed, Side, Snapshot, Summary, plan, remote, sync};

/// Where the base snapshot — what the two sides agreed on last time — is kept.
///
/// Under the daemon's root rather than in the vault, because it is derived state: deleting it costs
/// one conservative sync (everything looks changed) and not a single file.
fn state_dir(paths: &Paths) -> PathBuf {
    paths.root().join("sync")
}

/// What this machine calls itself in a conflict copy's name.
///
/// The setting first, then the host name, then a constant. The constant is the one that matters:
/// two machines that both fall back to it produce conflict copies neither user can tell apart, so
/// the screen asks for a name and this is only the floor.
#[must_use]
pub fn machine_name(settings: &summo_core::settings::Sync) -> String {
    let named = settings.machine.trim();
    if !named.is_empty() {
        return named.to_string();
    }
    std::env::var("HOSTNAME")
        .ok()
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "this-machine".to_string())
}

/// What a run or a dry run turned out to be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Whether anything was actually written. `false` for a dry run.
    pub applied: bool,
    /// The folder this ran against, echoed back so a screen can say which one it used.
    pub folder: String,
    pub machine: String,
    pub summary: Summary,
    /// Every step, so a dry run can show the list rather than only the count.
    pub steps: Vec<plan::Step>,
    /// Files both sides changed. Not an error: the user has two whole files and a decision.
    pub conflicts: Vec<Conflict>,
    /// Paths the remote offered that would have escaped the vault. Reported, never acted on.
    pub refused: Vec<String>,
}

/// One file both sides edited, and where the other version was put.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub path: String,
    pub copy: String,
}

/// Check a folder is one this daemon can sync through, before a passphrase is asked for.
///
/// Up front, so a typo is an answer to the request rather than a failure three steps in — and so
/// nobody types a passphrase into a screen that was going to refuse anyway.
///
/// # Errors
///
/// When the path is empty, relative, missing, or not a directory.
pub fn check(folder: &str) -> Result<PathBuf> {
    let trimmed = folder.trim();
    if trimmed.is_empty() {
        return Err(Error::msg(
            "sync.no_folder",
            "chưa chọn thư mục để đồng bộ".to_string(),
        ));
    }
    let path = PathBuf::from(trimmed);
    // A relative path is resolved against the daemon's working directory, which is wherever the
    // app happened to be launched from — not a thing the person typing it can predict.
    if !path.is_absolute() {
        return Err(Error::msg(
            "sync.relative_folder",
            format!("{trimmed} phải là đường dẫn tuyệt đối"),
        ));
    }
    if !path.exists() {
        return Err(Error::msg(
            "sync.not_found",
            format!("không thấy {trimmed}"),
        ));
    }
    if !path.is_dir() {
        return Err(Error::msg(
            "sync.not_a_directory",
            format!("{trimmed} không phải thư mục"),
        ));
    }
    Ok(path)
}

/// What a caller asked for.
pub struct Request {
    pub folder: String,
    pub passphrase: String,
    pub machine: String,
    /// Plan it and report, without writing anything. Worth having: the first sync of an existing
    /// vault moves every file, and somebody should be able to look before that happens.
    pub dry_run: bool,
}

/// Run one sync, or plan one.
///
/// Blocking: it reads and writes a folder of files. The caller puts it on a thread.
///
/// # Errors
///
/// When the folder cannot be opened, the salt cannot be written, the passphrase is empty, or the
/// remote manifest is malformed — which, given the manifest is sealed, usually means the
/// passphrase is wrong rather than the file.
pub fn run(paths: &Paths, request: &Request) -> Result<Report> {
    let folder = check(&request.folder)?;

    if request.passphrase.trim().is_empty() {
        return Err(Error::msg(
            "sync.no_passphrase",
            "cần mật khẩu đồng bộ".to_string(),
        ));
    }

    let state = state_dir(paths);
    std::fs::create_dir_all(&state)
        .map_err(|e| Error::Other(format!("cannot create {}: {e}", state.display())))?;

    let mut remote = remote::Directory::open(&folder)
        .map_err(|e| Error::Other(format!("cannot open {}: {e}", folder.display())))?;

    let salt = salt(&mut remote)?;
    let key = Key::derive(&request.passphrase, &salt)
        .map_err(|e| Error::Other(format!("cannot derive a key: {e}")))?;

    let machine = request.machine.clone();
    let folder_said = request.folder.trim().to_string();

    if request.dry_run {
        let local = Snapshot::scan(&paths.vault())
            .map_err(|e| Error::Other(format!("cannot read the vault: {e}")))?;
        let base = Snapshot::read(&state.join("base.json"));
        let theirs = remote_snapshot(&remote, &key)?;
        let plan = plan::plan(&local, &theirs, &base);

        return Ok(Report {
            applied: false,
            folder: folder_said,
            machine,
            summary: plan.summary(),
            steps: plan.steps.clone(),
            // A dry run writes nothing, so it produces no conflict copies — the files it *would*
            // reconcile are in `steps` as `merge`, which is the honest way to say so.
            conflicts: Vec::new(),
            refused: plan.refused.clone(),
        });
    }

    let outcome = sync(&paths.vault(), &state, &mut remote, &key, &machine)
        .map_err(|e| Error::Other(format!("{e}")))?;

    Ok(Report {
        applied: true,
        folder: folder_said,
        machine,
        summary: outcome.summary,
        // A run reports what it did through its summary and its conflicts; listing every step again
        // would be the same information twice, and on a first sync that is ten thousand lines.
        steps: Vec::new(),
        conflicts: outcome
            .conflicts
            .into_iter()
            .map(|c| Conflict {
                path: c.path,
                copy: c.copy,
            })
            .collect(),
        refused: outcome.refused,
    })
}

fn remote_snapshot(
    remote: &dyn Remote,
    key: &Key,
) -> Result<Snapshot> {
    let Some(bytes) = remote
        .manifest()
        .map_err(|e| Error::Other(format!("cannot read the remote manifest: {e}")))?
    else {
        return Ok(Snapshot::default());
    };
    let sealed = Sealed::from_bytes(&bytes)
        .map_err(|e| Error::Other(format!("the remote manifest is malformed: {e}")))?;
    // Passed through, not rewritten. `crypto` already says "sai passphrase, hoặc dữ liệu đã bị
    // sửa" — which is the honest pair, since the two are the same event to the cipher — and the
    // non-dry-run path inside `run` surfaces that same sentence. A second wording here would mean
    // one failure reading two different ways depending on which button was pressed.
    let plain = key.open("\u{0}summo-manifest", &sealed)?;
    let manifest: Manifest = serde_json::from_slice(&plain)
        .map_err(|e| Error::Other(format!("the remote manifest is malformed: {e}")))?;
    Ok(manifest.to_snapshot())
}

/// The vault's salt, from the remote, created there on the first sync.
///
/// On the remote because it belongs to the shared vault rather than to a machine: a salt per
/// machine means a different key per machine from the same passphrase, and the second machine to
/// sync cannot read anything the first wrote.
///
/// Not a secret, and stored in the clear. It exists so two people who chose the same passphrase do
/// not derive the same key, and so one precomputed table cannot cover both.
fn salt(remote: &mut dyn Remote) -> Result<Vec<u8>> {
    if let Some(existing) = remote
        .salt()
        .map_err(|e| Error::Other(format!("cannot read the sync salt: {e}")))?
        && existing.len() >= 8
    {
        return Ok(existing);
    }
    let fresh = crate::crypto::new_salt()
        .map_err(|e| Error::Other(format!("cannot create a sync salt: {e}")))?;
    remote
        .put_salt(&fresh)
        .map_err(|e| Error::Other(format!("cannot write the sync salt: {e}")))?;
    Ok(fresh.to_vec())
}

/// A step, as one line for a terminal.
#[must_use]
pub fn label(action: &Action) -> &'static str {
    match action {
        Action::Upload => "upload",
        Action::Download => "download",
        Action::Merge => "merge",
        Action::DeleteRemote => "delete there",
        Action::DeleteLocal => "delete here",
        Action::Resurrect {
            edited_on: Side::Local,
        } => "restore there",
        Action::Resurrect {
            edited_on: Side::Remote,
        } => "restore here",
    }
}

/// Whether the folder currently configured is usable, without asking for a passphrase.
///
/// For the screen: a folder that has been unplugged since it was chosen should say so where the
/// button is, not after somebody has typed a passphrase into it.
#[must_use]
pub fn folder_status(folder: Option<&str>) -> Option<std::result::Result<PathBuf, String>> {
    let folder = folder?;
    Some(check(folder).map_err(|e| e.to_string()))
}

/// Whether this vault has synced through a folder before.
///
/// Read from the base snapshot rather than from a setting, because the setting says what was
/// *chosen* and this says what has actually happened. The difference matters on the screen: the
/// first run of an existing vault uploads everything, and telling somebody that before they press
/// the button is the difference between a surprise and a decision.
#[must_use]
pub fn has_synced_before(paths: &Paths) -> bool {
    state_dir(paths).join("base.json").is_file()
}

/// Where the base snapshot lives, for a caller that wants to say so.
#[must_use]
pub fn base_snapshot(paths: &Paths) -> PathBuf {
    state_dir(paths).join("base.json")
}

/// A folder as a display string, with `~` put back if it is under the home directory.
#[must_use]
pub fn shorten(path: &Path) -> String {
    let shown = path.display().to_string();
    let Some(home) = std::env::var_os("HOME") else {
        return shown;
    };
    let home = home.to_string_lossy();
    if home.is_empty() {
        return shown;
    }
    match shown.strip_prefix(home.as_ref()) {
        Some(rest) => format!("~{rest}"),
        None => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nobody should type a passphrase into a screen that was going to refuse the folder anyway.
    #[test]
    fn a_folder_is_checked_before_a_passphrase_is_asked_for() {
        let err = check("").unwrap_err().to_string();
        assert!(err.contains("chưa chọn thư mục"), "{err}");

        let err = check("  ").unwrap_err().to_string();
        assert!(err.contains("chưa chọn thư mục"), "{err}");

        let err = check("relative/path").unwrap_err().to_string();
        assert!(err.contains("tuyệt đối"), "{err}");

        let err = check("/definitely/not/here/at/all")
            .unwrap_err()
            .to_string();
        assert!(err.contains("không thấy"), "{err}");
    }

    #[test]
    fn a_file_is_not_a_folder_to_sync_through() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "x").unwrap();

        let err = check(file.to_str().unwrap()).unwrap_err().to_string();
        assert!(err.contains("không phải thư mục"), "{err}");

        assert!(check(dir.path().to_str().unwrap()).is_ok());
    }

    /// Two machines that both fall back to the same name produce conflict copies their owners
    /// cannot tell apart, which is the one thing a conflict copy exists to prevent.
    #[test]
    fn a_machine_name_falls_back_rather_than_being_empty() {
        let mut settings = summo_core::settings::Sync::default();
        assert!(!machine_name(&settings).is_empty());

        settings.machine = "  macbook  ".into();
        assert_eq!(machine_name(&settings), "macbook");

        settings.machine = "   ".into();
        assert!(!machine_name(&settings).is_empty());
    }

    /// A dry run writes nothing, so it must not claim to have run.
    #[test]
    fn a_dry_run_is_refused_the_same_way_a_real_one_is() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        let err = run(
            &paths,
            &Request {
                folder: dir.path().display().to_string(),
                passphrase: "   ".into(),
                machine: "a".into(),
                dry_run: true,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("mật khẩu"), "{err}");
    }

    /// The whole path a daemon request takes, twice, through one folder.
    ///
    /// `run::tests` proves the merge; this proves the *entry point* — salt creation on first
    /// contact, key derivation from a passphrase, base snapshot placement — because that is the
    /// part each caller would otherwise have written for itself, and the part that was written
    /// once and reachable from one subcommand.
    #[test]
    fn a_file_written_on_one_machine_arrives_on_the_other() {
        let shared = tempfile::tempdir().unwrap();
        let here = tempfile::tempdir().unwrap();
        let there = tempfile::tempdir().unwrap();
        let (a, b) = (Paths::at(here.path()), Paths::at(there.path()));
        a.ensure().unwrap();
        b.ensure().unwrap();

        std::fs::write(a.vault().join("note.md"), "# Họp\n").unwrap();

        let ask = |machine: &str, dry_run: bool| Request {
            folder: shared.path().display().to_string(),
            passphrase: "mở cửa ra".into(),
            machine: machine.to_string(),
            dry_run,
        };

        // Nothing has been agreed yet, so the plan is "upload everything" — which is exactly what
        // somebody about to sync an existing vault for the first time should be able to see.
        let planned = run(&a, &ask("here", true)).unwrap();
        assert!(!planned.applied, "a dry run must not write");
        assert!(planned.summary.uploaded >= 1, "{:?}", planned.summary);
        assert!(!has_synced_before(&a), "a dry run must not leave a base");

        let pushed = run(&a, &ask("here", false)).unwrap();
        assert!(pushed.applied);
        assert!(has_synced_before(&a));

        run(&b, &ask("there", false)).unwrap();
        assert_eq!(
            std::fs::read_to_string(b.vault().join("note.md")).unwrap(),
            "# Họp\n"
        );
    }

    /// The same sentence whichever button was pressed.
    ///
    /// A wrong passphrase and tampered data are one event to the cipher, and `crypto` says so in
    /// one sentence. Planning and running reach that sentence by different routes — one opens the
    /// manifest here, one inside `run` — and a reader must not get two different explanations for
    /// the same typo depending on which they pressed.
    #[test]
    fn a_wrong_passphrase_says_so_rather_than_blaming_the_folder() {
        let shared = tempfile::tempdir().unwrap();
        let here = tempfile::tempdir().unwrap();
        let there = tempfile::tempdir().unwrap();
        let (a, b) = (Paths::at(here.path()), Paths::at(there.path()));
        a.ensure().unwrap();
        b.ensure().unwrap();
        std::fs::write(a.vault().join("note.md"), "# Họp\n").unwrap();

        run(
            &a,
            &Request {
                folder: shared.path().display().to_string(),
                passphrase: "đúng".into(),
                machine: "here".into(),
                dry_run: false,
            },
        )
        .unwrap();

        for dry_run in [true, false] {
            let err = run(
                &b,
                &Request {
                    folder: shared.path().display().to_string(),
                    passphrase: "sai".into(),
                    machine: "there".into(),
                    dry_run,
                },
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("sai passphrase"), "dry_run={dry_run}: {err}");
        }
    }

    /// The names a screen switches on.
    ///
    /// `Action` is flattened into `Step`, so the wire shape is `{path, action, …}` and the client
    /// matches on that string. A rename here is invisible to every Rust caller and silently turns
    /// one row of a plan into "unknown" for the reader — which is exactly the kind of break that
    /// ships. `apps/web/src/lib/sync.ts` has the other half of this list.
    #[test]
    fn a_step_is_on_the_wire_as_the_client_reads_it() {
        let step = |action: Action| {
            serde_json::to_value(plan::Step {
                path: "vault/note.md".into(),
                action,
            })
            .unwrap()
        };

        assert_eq!(step(Action::Upload)["action"], "upload");
        assert_eq!(step(Action::Download)["action"], "download");
        assert_eq!(step(Action::Merge)["action"], "merge");
        assert_eq!(step(Action::DeleteRemote)["action"], "delete_remote");
        assert_eq!(step(Action::DeleteLocal)["action"], "delete_local");

        let restored = step(Action::Resurrect {
            edited_on: Side::Local,
        });
        assert_eq!(restored["action"], "resurrect");
        assert_eq!(restored["edited_on"], "local");
        assert_eq!(restored["path"], "vault/note.md");
    }

    #[test]
    fn a_home_relative_folder_reads_as_one() {
        // Not `HOME` from the environment: a test that changes it races every other test in the
        // binary. This only checks the arithmetic of the prefix.
        let home = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned());
        let Some(home) = home.filter(|h| !h.is_empty()) else {
            return;
        };
        assert_eq!(shorten(Path::new(&format!("{home}/Sync/vault"))), "~/Sync/vault");
        assert_eq!(shorten(Path::new("/mnt/nas/vault")), "/mnt/nas/vault");
    }
}
