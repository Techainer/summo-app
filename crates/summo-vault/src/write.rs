//! Writing files into the vault.
//!
//! The vault is read by other programs — Obsidian, a sync client, a text editor with the file open
//! — while Summo is writing it. Every write therefore goes through a rename, so a reader sees one
//! complete version of a file or the other, never a half-written one.

use std::path::Path;

use summo_core::{Error, Result};

/// Write through a temporary file and rename over the target.
///
/// The rename is the point: it is atomic, so a reader either sees the previous complete file or the
/// new complete one, never a half-written meeting.
pub fn write_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Vault(format!("{} has no parent directory", path.display())))?;
    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;

    // Same directory, so the rename stays on one filesystem — across a boundary it would fall back
    // to a copy and stop being atomic.
    let temporary = path.with_extension("md.tmp");
    std::fs::write(&temporary, contents).map_err(|e| Error::io(&temporary, e))?;
    std::fs::rename(&temporary, path).map_err(|e| Error::io(path, e))?;
    Ok(())
}

