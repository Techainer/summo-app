//! Unpacking a model that ships as one file containing many.
//!
//! Every model in the registry until now has been a handful of loose files, and the store fetches
//! them by digest into a flat content-addressed blob directory. That covers ONNX exports and
//! tokenisers and stops exactly where the next thing we want begins: a **voice**. sherpa-onnx
//! publishes TTS voices as `.tar.bz2`, and a piper voice is not a file — it is a directory with an
//! `.onnx`, a `tokens.txt` and an `espeak-ng-data/` of several hundred phoneme tables that the
//! runtime opens by directory path. Listing those individually in a manifest is not a manifest.
//!
//! So a file entry may be an archive. It is fetched and digest-checked exactly like any other blob
//! — the identity of an archive is the identity of its bytes — and then unpacked once, beside the
//! blob, into `<digest>.d/`.
//!
//! ## The dangerous part
//!
//! An archive is a list of paths chosen by whoever built it, and this one arrived over the network
//! from an address a manifest named. `tar` will happily write `../../../.ssh/authorized_keys` if
//! asked, and a symlink member is a second way to ask. The registry checker already refuses "a file
//! name that could escape the model directory" for loose files; an extractor has the same problem
//! with more surface.
//!
//! Three rules, all enforced here rather than trusted from the archive:
//!
//! * every member path is relative, has no `..` component, and no root or prefix component;
//! * links — symbolic and hard — are refused outright rather than resolved, because a link is a
//!   path that means something different after it is written than it did when it was checked;
//! * the resolved destination is re-checked against the destination root after joining, so a
//!   component this code failed to imagine still cannot land outside it.
//!
//! Anything refused fails the install. Not skipped: a voice missing the one file that was a symlink
//! is a voice that loads and mispronounces, and "we quietly dropped part of your model" is a worse
//! outcome than "this did not install".

use std::path::{Component, Path, PathBuf};

use summo_core::{Error, Result};

/// How an archive is compressed. Taken from the manifest, never guessed from the file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Archive {
    /// `.tar.bz2` — what sherpa-onnx publishes its voices and several of its models as.
    TarBz2,
    /// `.tar.gz`, for publishers who use it.
    TarGz,
}

impl Archive {
    /// The extension a publisher would have written, for messages.
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::TarBz2 => "tar.bz2",
            Self::TarGz => "tar.gz",
        }
    }
}

/// Where an archive blob unpacks to: the blob's own path with `.d` appended.
///
/// Beside the blob rather than inside it, and derived rather than recorded, so the garbage collector
/// keeps working on digests alone — a blob nothing references takes its directory with it.
#[must_use]
pub fn unpacked_dir(blob: &Path) -> PathBuf {
    let mut dir = blob.as_os_str().to_os_string();
    dir.push(".d");
    PathBuf::from(dir)
}

/// Unpack `blob` into its directory, unless that has already happened.
///
/// Returns the directory. Idempotent: a completed extraction is marked, and an interrupted one is
/// thrown away and redone rather than being read as complete.
///
/// # Errors
///
/// When the archive cannot be read, or when any member is a link or names a path that would land
/// outside the destination.
pub fn unpack(blob: &Path, kind: Archive) -> Result<PathBuf> {
    let dir = unpacked_dir(blob);
    // A marker written last. Without it a process killed mid-extraction leaves a directory that
    // exists, is incomplete, and would be trusted forever after — the same reason the store writes
    // a model's manifest last.
    let done = dir.join(".complete");
    if done.is_file() {
        return Ok(dir);
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;

    let file = std::fs::File::open(blob).map_err(|e| Error::io(blob, e))?;
    let reader = std::io::BufReader::new(file);
    let result = match kind {
        Archive::TarBz2 => entries(tar::Archive::new(bzip2::read::BzDecoder::new(reader)), &dir),
        Archive::TarGz => entries(
            tar::Archive::new(flate2::read::GzDecoder::new(reader)),
            &dir,
        ),
    };

    if let Err(e) = result {
        // A refused archive leaves nothing behind. Half a model on disk is a model that will be
        // found by `resolve` and loaded.
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }

    std::fs::write(&done, b"").map_err(|e| Error::io(&done, e))?;
    Ok(dir)
}

fn entries<R: std::io::Read>(mut archive: tar::Archive<R>, dir: &Path) -> Result<()> {
    let members = archive
        .entries()
        .map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?;

    for member in members {
        let mut member = member.map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?;
        let path = member
            .path()
            .map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?
            .into_owned();

        // Links are refused, not followed. A symlink is a path that means one thing when it is
        // checked and another after something else is written next to it, and a hard link can point
        // at a file outside the destination that this process can read.
        let kind = member.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(Error::Config(format!(
                "refusing `{}`: the archive contains a link, which is a way to name a file outside it",
                path.display()
            )));
        }
        if !kind.is_file() && !kind.is_dir() {
            // Devices, fifos, and the rest have no business in a model. Refused rather than
            // ignored, for the reason in the module note.
            return Err(Error::Config(format!(
                "refusing `{}`: not a file or a directory",
                path.display()
            )));
        }

        let safe = within(&path)?;
        let target = dir.join(&safe);
        // Belt and braces. `within` rejects every component that could escape, and this catches a
        // component it failed to imagine — including whatever the platform does with a name this
        // code read as ordinary.
        if !target.starts_with(dir) {
            return Err(Error::Config(format!(
                "refusing `{}`: it resolves outside the model directory",
                path.display()
            )));
        }

        if kind.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| Error::io(&target, e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| Error::io(&target, e))?;
        std::io::copy(&mut member, &mut out).map_err(|e| Error::io(&target, e))?;
    }
    Ok(())
}

/// A member path reduced to something that cannot leave the directory it is joined to.
///
/// Rejects rather than sanitises. Stripping `..` out of a path silently changes which file the
/// archive said it contained, and a model that quietly installed a different layout than its
/// publisher shipped is worse than one that refused.
fn within(path: &Path) -> Result<PathBuf> {
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            // `./` is noise a tar writer adds; it means the same directory and cannot escape.
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::Config(format!(
                    "refusing `{}`: a member path may not be absolute or contain `..`",
                    path.display()
                )));
            }
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(Error::Config(
            "refusing an archive member with an empty path".into(),
        ));
    }
    Ok(safe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a `.tar.gz` from `(path, contents)` pairs, so a test can say what it means.
    fn targz(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (name, body) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, *body).unwrap();
        }
        let raw = tar.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap()
    }

    /// A tar built byte by byte, because `tar::Builder` refuses to write the paths this module
    /// exists to refuse — and so does every other well-behaved writer. An archive that attacks you
    /// was not produced by a well-behaved writer, so the test cannot use one either.
    ///
    /// Members are `(path, typeflag, linkname, body)`. One stream, one set of end blocks, gzipped
    /// once: two archives concatenated would stop the reader at the first one's end markers, and a
    /// test whose hostile member is never reached passes for the wrong reason.
    fn hostile(members: &[(&str, u8, &str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, kind, link, body) in members {
            let mut header = [0u8; 512];
            let put =
                |h: &mut [u8; 512], at: usize, v: &[u8]| h[at..at + v.len()].copy_from_slice(v);
            put(&mut header, 0, name.as_bytes());
            put(&mut header, 100, b"0000644\x00");
            put(&mut header, 108, b"0000000\x00");
            put(&mut header, 116, b"0000000\x00");
            put(
                &mut header,
                124,
                format!("{:011o}\x00", body.len()).as_bytes(),
            );
            put(&mut header, 136, b"00000000000\x00");
            header[148..156].fill(b' '); // the checksum field counts as spaces while summing
            header[156] = *kind;
            put(&mut header, 157, link.as_bytes());
            put(&mut header, 257, b"ustar\x0000");

            let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
            put(&mut header, 148, format!("{sum:06o}\x00 ").as_bytes());

            out.extend_from_slice(&header);
            out.extend_from_slice(body);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.extend_from_slice(&[0u8; 1024]);

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&out).unwrap();
        gz.finish().unwrap()
    }

    fn blob(tmp: &Path, bytes: &[u8]) -> PathBuf {
        let at = tmp.join("a".repeat(64));
        std::fs::write(&at, bytes).unwrap();
        at
    }

    #[test]
    fn a_voice_unpacks_to_a_directory_the_runtime_can_open() {
        let tmp = tempfile::tempdir().unwrap();
        let at = blob(
            tmp.path(),
            &targz(&[
                ("voice/model.onnx", b"weights"),
                ("voice/tokens.txt", b"tokens"),
                ("voice/espeak-ng-data/phontab", b"tables"),
            ]),
        );

        let dir = unpack(&at, Archive::TarGz).unwrap();
        assert_eq!(
            std::fs::read(dir.join("voice/model.onnx")).unwrap(),
            b"weights"
        );
        assert!(dir.join("voice/espeak-ng-data/phontab").is_file());
    }

    /// The attack this file exists to refuse. A manifest names an address; the bytes at that
    /// address are a list of paths somebody else chose.
    #[test]
    fn a_member_that_climbs_out_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let at = blob(tmp.path(), &hostile(&[("../escaped.txt", b'0', "", b"no")]));

        let Err(err) = unpack(&at, Archive::TarGz) else {
            panic!("an archive wrote outside its own directory")
        };
        assert!(err.to_string().contains(".."), "got: {err}");
        assert!(
            !tmp.path().join("escaped.txt").exists(),
            "the file landed anyway"
        );
    }

    /// And nothing of a refused archive is left behind, because `resolve` would find it and a
    /// runtime would load it.
    #[test]
    fn a_refused_archive_leaves_no_half_a_model() {
        let tmp = tempfile::tempdir().unwrap();
        let at = blob(
            tmp.path(),
            &hostile(&[
                ("voice/model.onnx", b'0', "", b"weights" as &[u8]),
                ("../escaped.txt", b'0', "", b"no"),
            ]),
        );

        assert!(unpack(&at, Archive::TarGz).is_err());
        assert!(
            !unpacked_dir(&at).exists(),
            "the first member survived the refusal"
        );
    }

    /// A link is a path that means one thing when it is checked and another after it is written.
    #[test]
    fn a_symlink_member_is_refused_rather_than_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let at = blob(
            tmp.path(),
            &hostile(&[("voice/keys", b'2', "/etc/passwd", b"")]),
        );

        let Err(err) = unpack(&at, Archive::TarGz) else {
            panic!("an archive planted a symlink")
        };
        assert!(err.to_string().contains("link"), "got: {err}");
    }

    /// An absolute member is the same attack spelled differently.
    #[test]
    fn an_absolute_member_is_refused() {
        assert!(within(Path::new("/etc/passwd")).is_err());
        assert!(within(Path::new("voice/../../etc/passwd")).is_err());
        // And the ordinary shape a tar writer produces still works.
        assert_eq!(
            within(Path::new("./voice/model.onnx")).unwrap(),
            PathBuf::from("voice/model.onnx")
        );
    }

    /// Unpacking twice does the work once. Installing a model that is already installed is an
    /// ordinary thing to do — `summo pull` on a model you have, a second window opening the
    /// catalogue — and re-extracting several hundred phoneme tables each time is not free.
    #[test]
    fn unpacking_is_done_once() {
        let tmp = tempfile::tempdir().unwrap();
        let at = blob(tmp.path(), &targz(&[("voice/model.onnx", b"weights")]));

        let dir = unpack(&at, Archive::TarGz).unwrap();
        std::fs::write(dir.join("voice/model.onnx"), b"touched").unwrap();
        let again = unpack(&at, Archive::TarGz).unwrap();

        assert_eq!(dir, again);
        assert_eq!(
            std::fs::read(again.join("voice/model.onnx")).unwrap(),
            b"touched",
            "it extracted again over a completed directory"
        );
    }

    /// The real thing: a published piper voice, 397 members and an `espeak-ng-data/` tree.
    ///
    /// Every other test here builds its own archive, which proves the rules and proves nothing
    /// about the format as a publisher actually writes it. Skipped unless pointed at a download,
    /// the same way the decoder tests are.
    #[test]
    fn a_published_voice_unpacks_whole() {
        let Ok(at) = std::env::var("SUMMO_TEST_VOICE_ARCHIVE") else {
            eprintln!("skipping: set SUMMO_TEST_VOICE_ARCHIVE");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let blob = tmp.path().join("b".repeat(64));
        std::fs::copy(&at, &blob).unwrap();

        let dir = unpack(&blob, Archive::TarBz2).unwrap();
        let root = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .expect("the voice unpacked to no directory");

        assert!(root.join("tokens.txt").is_file(), "no tokens.txt");
        assert!(
            std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .any(|e| e.path().extension().is_some_and(|x| x == "onnx")),
            "no .onnx in the voice"
        );
        let phonemes = root.join("espeak-ng-data");
        assert!(
            phonemes.is_dir() && std::fs::read_dir(&phonemes).unwrap().count() > 10,
            "espeak-ng-data is what makes this a directory rather than a file"
        );
    }

    /// An extraction killed halfway leaves a directory with no marker in it, and that must read as
    /// "not done" rather than as a model.
    #[test]
    fn an_interrupted_extraction_is_redone_rather_than_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        let at = blob(tmp.path(), &targz(&[("voice/model.onnx", b"weights")]));

        let dir = unpacked_dir(&at);
        std::fs::create_dir_all(dir.join("voice")).unwrap();
        std::fs::write(dir.join("voice/model.onnx"), b"half").unwrap();

        let done = unpack(&at, Archive::TarGz).unwrap();
        assert_eq!(
            std::fs::read(done.join("voice/model.onnx")).unwrap(),
            b"weights",
            "a directory with no marker was read as a finished extraction"
        );
    }
}
