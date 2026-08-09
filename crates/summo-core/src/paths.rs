//! Where Summo keeps things on disk.
//!
//! The layout is deliberately boring and inspectable: a Markdown vault the user can open in
//! Obsidian, a content-addressed model blob store, and a SQLite index that can be deleted and
//! rebuilt from the vault at any time.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Override for the whole data directory. Set by tests, portable installs and CI.
pub const ENV_DATA_DIR: &str = "SUMMO_HOME";
/// Override for the registry root (a URL or a `file://` path).
pub const ENV_REGISTRY: &str = "SUMMO_REGISTRY";

/// Resolved application directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// Resolve from `SUMMO_HOME`, else the platform data directory.
    ///
    /// * Linux: `~/.local/share/summo` (XDG)
    /// * macOS: `~/Library/Application Support/Summo`
    /// * Windows: `%APPDATA%\Summo\data`
    pub fn discover() -> Result<Self> {
        if let Some(dir) = std::env::var_os(ENV_DATA_DIR) {
            return Ok(Self::at(PathBuf::from(dir)));
        }
        let dirs = directories::ProjectDirs::from("app", "Summo", "Summo")
            .ok_or_else(|| Error::Config("cannot determine a home directory".into()))?;
        Ok(Self::at(dirs.data_dir().to_path_buf()))
    }

    /// Root an instance at an explicit directory.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Markdown vault — the source of truth. Everything else is derived and rebuildable.
    #[must_use]
    pub fn vault(&self) -> PathBuf {
        self.root.join("vault")
    }

    #[must_use]
    pub fn meetings(&self) -> PathBuf {
        self.vault().join("meetings")
    }

    #[must_use]
    pub fn notes(&self) -> PathBuf {
        self.vault().join("notes")
    }

    #[must_use]
    pub fn people(&self) -> PathBuf {
        self.vault().join("people")
    }

    #[must_use]
    pub fn attachments(&self) -> PathBuf {
        self.vault().join("attachments")
    }

    /// Recorded audio, one directory per meeting, one Opus file per lane.
    #[must_use]
    pub fn audio(&self) -> PathBuf {
        self.root.join("audio")
    }

    #[must_use]
    pub fn audio_for(&self, meeting: &crate::MeetingId) -> PathBuf {
        self.audio().join(meeting.as_str())
    }

    /// Derived search index. Safe to delete — rebuilt from the vault.
    #[must_use]
    pub fn index_db(&self) -> PathBuf {
        self.root.join("index.db")
    }

    #[must_use]
    pub fn models(&self) -> PathBuf {
        self.root.join("models")
    }

    /// Content-addressed blobs, shared between models that reference the same file.
    #[must_use]
    pub fn blobs(&self) -> PathBuf {
        self.models().join("blobs").join("sha256")
    }

    /// Path for a blob digest. Sharded by the first two hex chars to keep directories small.
    ///
    /// Returns an error for anything that is not a 64-char lowercase hex digest, since this value
    /// reaches the filesystem and manifests are fetched over the network.
    pub fn blob(&self, sha256: &str) -> Result<PathBuf> {
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Config(format!(
                "`{sha256}` is not a sha256 hex digest"
            )));
        }
        if sha256.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(Error::Config("sha256 digests must be lowercase hex".into()));
        }
        Ok(self.blobs().join(&sha256[..2]).join(sha256))
    }

    /// Installed model manifests, keyed by model id.
    #[must_use]
    pub fn manifests(&self) -> PathBuf {
        self.models().join("manifests")
    }

    #[must_use]
    pub fn voices(&self) -> PathBuf {
        self.root.join("voices")
    }

    #[must_use]
    pub fn skills(&self) -> PathBuf {
        self.root.join("skills")
    }

    #[must_use]
    pub fn hw_profile(&self) -> PathBuf {
        self.root.join("hw.json")
    }

    #[must_use]
    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// Partial downloads. Kept out of the blob store so an interrupted transfer is never mistaken
    /// for a verified one.
    #[must_use]
    pub fn downloads(&self) -> PathBuf {
        self.models().join("partial")
    }

    /// Create every directory the app expects. Idempotent.
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            self.meetings(),
            self.notes(),
            self.people(),
            self.attachments(),
            self.audio(),
            self.blobs(),
            self.manifests(),
            self.downloads(),
            self.voices(),
            self.skills(),
        ] {
            std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_creates_the_whole_layout() {
        let tmp = std::env::temp_dir().join(format!("summo-paths-{}", uuid::Uuid::now_v7()));
        let paths = Paths::at(&tmp);
        paths.ensure().unwrap();
        assert!(paths.meetings().is_dir());
        assert!(paths.blobs().is_dir());
        assert!(paths.downloads().is_dir());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn blob_paths_are_sharded() {
        let digest = "a".repeat(64);
        let p = Paths::at("/data").blob(&digest).unwrap();
        assert_eq!(
            p,
            PathBuf::from("/data/models/blobs/sha256/aa").join(&digest)
        );
    }

    #[test]
    fn blob_rejects_anything_that_is_not_a_digest() {
        let paths = Paths::at("/data");
        assert!(paths.blob("../../etc/passwd").is_err());
        assert!(paths.blob("short").is_err());
        assert!(
            paths.blob(&"A".repeat(64)).is_err(),
            "uppercase would alias on case-insensitive fs"
        );
        assert!(
            paths.blob(&"g".repeat(64)).is_err(),
            "non-hex must be rejected"
        );
    }

    #[test]
    fn env_override_wins() {
        // SAFETY: single-threaded test process section; no other thread reads the environment here.
        unsafe { std::env::set_var(ENV_DATA_DIR, "/tmp/summo-override") };
        let paths = Paths::discover().unwrap();
        unsafe { std::env::remove_var(ENV_DATA_DIR) };
        assert_eq!(paths.root(), Path::new("/tmp/summo-override"));
    }
}
