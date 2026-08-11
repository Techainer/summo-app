//! Where the sealed blobs go.
//!
//! A trait, with a plain-directory implementation. That is not scaffolding for a "real" one later —
//! it is a first-class answer. A user who points sync at a folder on a NAS, a Dropbox directory, or
//! a USB stick gets working multi-machine sync with no account and no relay, and the paid tier
//! becomes a hosted convenience rather than the only way to use the feature. It also means every
//! test in [`crate::run`] exercises the real code path instead of a mock.
//!
//! ## What a remote has to do, and what it does not
//!
//! It stores bytes under an opaque id and lists what it holds. It does **not** merge, resolve, or
//! understand a vault: all of that is on this side, where the key is. A remote that could do any of
//! it would be a remote that could read the vault.
//!
//! The manifest is written last, after every blob it names. A run interrupted halfway leaves blobs
//! nothing points at — wasted space, collectable later — rather than a manifest pointing at blobs
//! that were never uploaded, which is a vault the other machine cannot open.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use summo_core::{Error, Result};

use crate::snapshot::{Entry, Snapshot};

/// What one machine last published: the vault's shape, under encrypted names.
///
/// The manifest itself is sealed too. Unencrypted it would list every path in the vault, which is
/// most of what a relay would want to know.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Vault-relative path to its content hash and size. The plaintext view, only ever held by a
    /// machine that has the key.
    #[serde(default)]
    pub files: BTreeMap<String, Entry>,
}

impl Manifest {
    #[must_use]
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            files: snapshot.files.clone(),
        }
    }

    #[must_use]
    pub fn to_snapshot(&self) -> Snapshot {
        Snapshot {
            files: self.files.clone(),
        }
    }
}

/// Somewhere sealed blobs can be kept.
///
/// Synchronous on purpose. Sync runs off the recording path in its own task, the work is dominated
/// by IO the caller wants to sequence anyway, and a synchronous trait can be implemented by a
/// directory in ten lines — which is what makes the local backend a real feature rather than a
/// test double.
pub trait Remote: Send {
    /// The salt every machine derives its key from, or `None` if nothing has synced here yet.
    ///
    /// It lives here rather than beside each vault because it belongs to *this shared vault*, not
    /// to a machine. Keeping a copy per machine meant each one derived a different key from the
    /// same passphrase, and the second machine to sync could not read a byte the first had written.
    ///
    /// It is not a secret. It exists so two people who chose the same passphrase do not end up with
    /// the same key, and so one precomputed table cannot cover both.
    fn salt(&self) -> Result<Option<Vec<u8>>>;

    fn put_salt(&mut self, salt: &[u8]) -> Result<()>;

    /// The manifest last published, or `None` if this vault has never synced here.
    fn manifest(&self) -> Result<Option<Vec<u8>>>;

    /// Publish the manifest. Called last, after every blob it names.
    fn put_manifest(&mut self, sealed: &[u8]) -> Result<()>;

    fn get(&self, id: &str) -> Result<Option<Vec<u8>>>;

    fn put(&mut self, id: &str, sealed: &[u8]) -> Result<()>;

    fn delete(&mut self, id: &str) -> Result<()>;

    /// A name for logs and for the conflict copies this remote's edits produce.
    fn name(&self) -> String {
        "remote".to_string()
    }
}

/// A directory: a NAS mount, a synced folder, a USB stick.
pub struct Directory {
    root: PathBuf,
}

impl Directory {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("blobs")).map_err(|e| Error::io(&root, e))?;
        Ok(Self { root })
    }

    /// Where a blob lives.
    ///
    /// Ids come from [`crate::Key::id_for`], so they are always 64 hex characters — but this is the
    /// boundary where a name from elsewhere becomes a path, so it is checked rather than trusted.
    fn blob(&self, id: &str) -> Result<PathBuf> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::msg(
                "sync.bad_id",
                format!("`{id}` is not a blob id"),
            ));
        }
        Ok(self.root.join("blobs").join(id))
    }
}

impl Remote for Directory {
    fn salt(&self) -> Result<Option<Vec<u8>>> {
        let path = self.root.join("salt");
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(&path, e)),
        }
    }

    fn put_salt(&mut self, salt: &[u8]) -> Result<()> {
        write_atomically(&self.root.join("salt"), salt)
    }

    fn manifest(&self) -> Result<Option<Vec<u8>>> {
        let path = self.root.join("manifest.bin");
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(&path, e)),
        }
    }

    fn put_manifest(&mut self, sealed: &[u8]) -> Result<()> {
        write_atomically(&self.root.join("manifest.bin"), sealed)
    }

    fn get(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let path = self.blob(id)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(&path, e)),
        }
    }

    fn put(&mut self, id: &str, sealed: &[u8]) -> Result<()> {
        write_atomically(&self.blob(id)?, sealed)
    }

    fn delete(&mut self, id: &str) -> Result<()> {
        let path = self.blob(id)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Already gone is the desired state, not a failure. Two machines both noticing the same
            // deletion is ordinary.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(&path, e)),
        }
    }

    fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "directory".to_string())
    }
}

/// Write via a temporary file and rename.
///
/// A manifest half-written when the power goes is a vault the other machine cannot open. Rename is
/// atomic on every filesystem this runs on, so a reader sees either the old file or the new one.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let temporary = path.with_extension("writing.tmp");
    std::fs::write(&temporary, bytes).map_err(|e| Error::io(&temporary, e))?;
    std::fs::rename(&temporary, path).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    #[test]
    fn a_fresh_directory_has_no_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let remote = Directory::open(tmp.path()).unwrap();
        assert!(remote.manifest().unwrap().is_none());
    }

    #[test]
    fn a_blob_survives_a_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut remote = Directory::open(tmp.path()).unwrap();

        remote.put(&id(1), b"sealed bytes").unwrap();
        assert_eq!(remote.get(&id(1)).unwrap().unwrap(), b"sealed bytes");
    }

    #[test]
    fn a_blob_that_is_not_there_is_none_rather_than_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            Directory::open(tmp.path())
                .unwrap()
                .get(&id(9))
                .unwrap()
                .is_none()
        );
    }

    /// Each machine keeping its own salt meant each derived a different key from the same
    /// passphrase, and the second one to sync could not read a byte the first had written.
    #[test]
    fn the_salt_belongs_to_the_remote_so_every_machine_shares_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut remote = Directory::open(tmp.path()).unwrap();
        assert!(remote.salt().unwrap().is_none(), "a fresh remote has none");

        remote.put_salt(b"0123456789abcdef").unwrap();
        assert_eq!(remote.salt().unwrap().unwrap(), b"0123456789abcdef");

        // A second machine opening the same folder sees the same salt.
        let second = Directory::open(tmp.path()).unwrap();
        assert_eq!(second.salt().unwrap().unwrap(), b"0123456789abcdef");
    }

    #[test]
    fn a_manifest_survives_a_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut remote = Directory::open(tmp.path()).unwrap();

        remote.put_manifest(b"sealed manifest").unwrap();
        assert_eq!(remote.manifest().unwrap().unwrap(), b"sealed manifest");
    }

    #[test]
    fn writing_a_manifest_twice_replaces_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut remote = Directory::open(tmp.path()).unwrap();
        remote.put_manifest(b"first").unwrap();
        remote.put_manifest(b"second").unwrap();
        assert_eq!(remote.manifest().unwrap().unwrap(), b"second");
    }

    /// Two machines both noticing the same deletion is ordinary, not a failure.
    #[test]
    fn deleting_something_already_gone_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let mut remote = Directory::open(tmp.path()).unwrap();
        assert!(remote.delete(&id(3)).is_ok());
    }

    #[test]
    fn a_deleted_blob_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let mut remote = Directory::open(tmp.path()).unwrap();
        remote.put(&id(4), b"x").unwrap();
        remote.delete(&id(4)).unwrap();
        assert!(remote.get(&id(4)).unwrap().is_none());
    }

    /// Ids come from a keyed hash, but this is the boundary where a name from elsewhere becomes a
    /// path — so it is checked rather than trusted.
    #[test]
    fn an_id_that_is_not_a_hash_cannot_reach_the_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        let mut remote = Directory::open(tmp.path()).unwrap();

        for bad in ["../../etc/passwd", "", "short", &"z".repeat(64)] {
            assert!(remote.get(bad).is_err(), "read allowed for {bad:?}");
            assert!(remote.put(bad, b"x").is_err(), "write allowed for {bad:?}");
            assert!(remote.delete(bad).is_err(), "delete allowed for {bad:?}");
        }
    }

    /// A half-written manifest is a vault the other machine cannot open.
    #[test]
    fn a_write_leaves_no_partial_file_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let mut remote = Directory::open(tmp.path()).unwrap();
        remote.put_manifest(b"whole").unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn a_manifest_is_the_snapshot_it_came_from() {
        let snapshot = Snapshot {
            files: [(
                "a.md".to_string(),
                Entry {
                    hash: "h".into(),
                    size: 1,
                    modified: 2,
                },
            )]
            .into_iter()
            .collect(),
        };
        assert_eq!(Manifest::from_snapshot(&snapshot).to_snapshot(), snapshot);
    }
}
