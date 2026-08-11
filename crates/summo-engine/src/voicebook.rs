//! The voice book, held in memory.
//!
//! This is the other half of the storage answer in ADR 0003, and the two halves are different
//! because the two datasets are different.
//!
//! The **history** — every utterance ever attributed, with its vector — is large and cold. Two
//! hundred thousand vectors is 147 MB, and it is touched only when somebody names a voice and the
//! past has to be re-swept. It lives in one binary file per meeting and is read on demand.
//!
//! The **book** — the people, and the handful of centroids that describe how each of them sounds —
//! is small and hot. Two hundred people is about 1,600 vectors, 1.2 MB, and it is consulted for
//! *every utterance of every meeting*. Reading 1.2 MB off disk to answer "who just spoke?" several
//! hundred times an hour would be absurd, so it is loaded once and kept.
//!
//! That is the whole design. No database, because the hot set fits in memory and the cold set is
//! read sequentially — the two access patterns a filesystem is already good at.
//!
//! Writes go through to disk immediately rather than being batched. A correction is a user action
//! at human speed, and losing one to a crash would be worse than the milliseconds saved.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use summo_core::Result;
use summo_diar::VoiceBook;

/// A voice book shared across the daemon, loaded once.
#[derive(Clone)]
pub struct SharedBook {
    inner: Arc<RwLock<VoiceBook>>,
    path: PathBuf,
}

impl SharedBook {
    /// Read the book from `voices_dir`, treating a missing file as an empty book.
    pub fn load(voices_dir: &std::path::Path) -> Result<Self> {
        let path = voices_dir.join("book.json");
        Ok(Self {
            inner: Arc::new(RwLock::new(VoiceBook::load(&path)?)),
            path,
        })
    }

    /// Read from the book without touching the disk.
    ///
    /// This is the identification path. It takes a read lock, so concurrent utterances from
    /// different lanes do not serialise against each other.
    pub fn read<T>(&self, f: impl FnOnce(&VoiceBook) -> T) -> T {
        f(&self.inner.read())
    }

    /// Modify the book and persist the result.
    ///
    /// The write lock is held across the save so a reader never observes a book that has been
    /// changed in memory but not on disk — which after a crash would be a book that disagrees with
    /// the transcripts it produced.
    pub fn write<T>(&self, f: impl FnOnce(&mut VoiceBook) -> Result<T>) -> Result<T> {
        let mut book = self.inner.write();
        let out = f(&mut book)?;
        book.save()?;
        Ok(out)
    }

    /// Discard the in-memory copy and read the file again.
    ///
    /// Needed because the vault is the user's: they may edit or restore `book.json` underneath a
    /// running daemon, and a cache that cannot be invalidated would serve the old answer forever.
    pub fn reload(&self) -> Result<()> {
        let fresh = VoiceBook::load(&self.path)?;
        *self.inner.write() = fresh;
        Ok(())
    }

    /// Where the book is stored.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const NGOC: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const BINH: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

    #[test]
    fn a_fresh_vault_loads_an_empty_book() {
        let dir = TempDir::new().unwrap();
        let book = SharedBook::load(dir.path()).expect("load");
        assert_eq!(book.read(summo_diar::VoiceBook::len), 0);
    }

    #[test]
    fn a_write_is_visible_in_memory_and_on_disk() {
        let dir = TempDir::new().unwrap();
        let book = SharedBook::load(dir.path()).expect("load");

        book.write(|b| b.enroll("Ngọc", &[NGOC.to_vec()], true))
            .expect("enroll");

        assert_eq!(book.read(summo_diar::VoiceBook::len), 1);
        // A second daemon reading the same vault must see it, which is what going through to disk
        // rather than batching is for.
        let reopened = SharedBook::load(dir.path()).expect("reload");
        assert_eq!(reopened.read(summo_diar::VoiceBook::len), 1);
    }

    #[test]
    fn clones_share_one_book_rather_than_diverging() {
        let dir = TempDir::new().unwrap();
        let book = SharedBook::load(dir.path()).expect("load");
        let other = book.clone();

        book.write(|b| b.enroll("Ngọc", &[NGOC.to_vec()], true))
            .expect("enroll");
        assert_eq!(
            other.read(summo_diar::VoiceBook::len),
            1,
            "a clone is the same book, not a copy of it"
        );
    }

    #[test]
    fn a_failed_write_does_not_leave_a_half_change_on_disk() {
        let dir = TempDir::new().unwrap();
        let book = SharedBook::load(dir.path()).expect("load");
        book.write(|b| b.enroll("Ngọc", &[NGOC.to_vec()], true))
            .expect("enroll");

        // An empty name is refused by the book itself.
        assert!(
            book.write(|b| b.enroll("  ", &[BINH.to_vec()], true))
                .is_err()
        );

        let reopened = SharedBook::load(dir.path()).expect("reload");
        assert_eq!(reopened.read(summo_diar::VoiceBook::len), 1);
    }

    #[test]
    fn reloading_picks_up_a_file_changed_underneath_us() {
        let dir = TempDir::new().unwrap();
        let book = SharedBook::load(dir.path()).expect("load");
        assert_eq!(book.read(summo_diar::VoiceBook::len), 0);

        // Somebody restores a backup while the daemon is running.
        let other = SharedBook::load(dir.path()).expect("load");
        other
            .write(|b| b.enroll("Ngọc", &[NGOC.to_vec()], true))
            .expect("enroll");

        assert_eq!(
            book.read(summo_diar::VoiceBook::len),
            0,
            "the cache is a cache; it does not notice on its own"
        );
        book.reload().expect("reload");
        assert_eq!(book.read(summo_diar::VoiceBook::len), 1);
    }

    #[test]
    fn identification_reads_without_touching_the_disk() {
        let dir = TempDir::new().unwrap();
        let book = SharedBook::load(dir.path()).expect("load");
        book.write(|b| b.enroll("Ngọc", &[NGOC.to_vec()], true))
            .expect("enroll");

        // Delete the file: a cached book must still answer, because it is not reading it.
        std::fs::remove_file(book.path()).expect("remove");
        let matched = book.read(|b| b.identify(NGOC.as_ref()).person().map(str::to_string));
        assert_eq!(matched.as_deref(), Some("ngoc"));
    }
}
