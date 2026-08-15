//! Pictures, in the vault, beside the notes that use them.
//!
//! A note that can hold an image has to put the image somewhere, and there are only two honest
//! answers. Inline it into the Markdown as a data URI — which turns a 400 kB screenshot into 550 kB
//! of base64 on a line no editor can wrap, in a file the user greps — or write it to a file beside
//! the note and link to it. This is the second.
//!
//! ## Where the link points
//!
//! `attachments/<name>`, relative to the vault root. Not to the note: notes can be filed in folders
//! and recordings live in a different tree entirely, so a path relative to the document would be a
//! different number of `../` for every page and would break the moment somebody refiled one. The
//! vault root is the one thing every document shares.
//!
//! Obsidian resolves a Markdown link that fails relative lookup against the vault root, which is why
//! this shape opens there as well as here. `attachments/` is also where [`summo_diar::voices`]
//! already puts a person's photograph, so a vault has one folder of pictures rather than two.
//!
//! ## The name is the content
//!
//! A hash of the bytes, plus the extension the bytes say they are. Two decisions come out of that
//! for free: pasting the same screenshot into four notes writes one file, and no upload can ever
//! overwrite another — which a name taken from the client could, and would be the first thing
//! anybody tried.
//!
//! ## What is refused
//!
//! The format is read from the file's own magic bytes and never from what the client said. The
//! interface is served from the daemon's origin, so a file served back under that origin can run
//! script in the app if the browser is told it is HTML or SVG — and an SVG *is* a script container.
//! Anything that is not one of the four raster formats below is refused rather than stored under a
//! generic type: a file whose bytes nobody recognises is not one to hand back to a browser.

use std::path::{Path, PathBuf};

use summo_core::{Error, Result, paths::Paths};

/// The largest picture a note may hold, in bytes.
///
/// A vault is a folder somebody syncs. This is generous for a screenshot and a photograph and
/// refuses a video somebody dragged in by accident, which is the case worth having a limit for.
pub const MAX_BYTES: usize = 12 * 1024 * 1024;

/// A format the vault will store, and what a browser must be told it is.
struct Format {
    extension: &'static str,
    content_type: &'static str,
    /// A prefix the bytes must start with, or `None` when the check is more than a prefix.
    magic: &'static [u8],
}

/// What the bytes are allowed to be.
///
/// WebP is checked as `RIFF....WEBP`, which is a prefix and a window rather than one prefix, so it
/// is handled beside this table in [`sniff`].
const FORMATS: [Format; 3] = [
    Format {
        extension: "png",
        content_type: "image/png",
        magic: b"\x89PNG\r\n\x1a\n",
    },
    Format {
        extension: "jpg",
        content_type: "image/jpeg",
        magic: b"\xff\xd8\xff",
    },
    Format {
        extension: "gif",
        content_type: "image/gif",
        magic: b"GIF8",
    },
];

/// What these bytes are, or nothing when they are not a picture this vault stores.
fn sniff(bytes: &[u8]) -> Option<&'static Format> {
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(&WEBP);
    }
    FORMATS
        .iter()
        .find(|format| bytes.starts_with(format.magic))
}

const WEBP: Format = Format {
    extension: "webp",
    content_type: "image/webp",
    magic: b"RIFF",
};

/// Store a picture and return the path a note should link to, relative to the vault.
///
/// Writing the same bytes twice is not an error and does not write twice: the name is the content,
/// so the second upload finds the file already there. That is what makes pasting one screenshot
/// into a week of notes cost one file.
pub fn store(paths: &Paths, bytes: &[u8]) -> Result<String> {
    if bytes.is_empty() {
        return Err(Error::msg("attachment.empty", "tệp rỗng"));
    }
    if bytes.len() > MAX_BYTES {
        return Err(Error::msg(
            "attachment.too_large",
            format!("ảnh quá lớn, tối đa {} MB", MAX_BYTES / 1024 / 1024),
        ));
    }
    let Some(format) = sniff(bytes) else {
        return Err(Error::msg(
            "attachment.unsupported",
            "chỉ nhận ảnh PNG, JPEG, GIF hoặc WebP",
        ));
    };

    // Sixteen bytes of the digest. This names a file in one person's vault rather than addressing
    // content on a network, and a 128-bit name is already far past the point where two pictures
    // could collide by accident.
    let digest = blake3::hash(bytes);
    let name = format!("{}.{}", &digest.to_hex()[..32], format.extension);

    let dir = paths.attachments();
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let path = dir.join(&name);
    if !path.exists() {
        crate::write::write_atomically(&path, bytes)?;
    }
    Ok(format!("attachments/{name}"))
}

/// Where a stored picture is, and what to serve it as.
///
/// The name is validated rather than joined and hoped for. `..`, an absolute path and a nested
/// directory are all rejected before anything touches the filesystem, because this is reached from
/// a URL — and `attachments/../../../etc/passwd` is the request somebody makes on the first day.
pub fn locate(paths: &Paths, name: &str) -> Result<(PathBuf, &'static str)> {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return Err(missing(name));
    };
    let named = stem.len() == 32 && stem.bytes().all(|b| b.is_ascii_hexdigit());
    let Some(format) = FORMATS
        .iter()
        .chain(std::iter::once(&WEBP))
        .find(|format| format.extension == extension)
    else {
        return Err(missing(name));
    };
    if !named {
        return Err(missing(name));
    }

    let path = paths.attachments().join(name);
    if !path.is_file() {
        return Err(missing(name));
    }
    Ok((path, format.content_type))
}

/// Whether a vault-relative path names an attachment, for a caller holding a link from a note.
///
/// One path segment with a stem and an extension. `attachments/` alone, `attachments/.` and
/// anything with a directory in it are not files, and the strictness matters because the answer is
/// used to decide what a sweep may delete.
#[must_use]
pub fn is_attachment(link: &str) -> bool {
    let Some(name) = link.strip_prefix("attachments/") else {
        return false;
    };
    if name.contains('/') || name.contains('\\') {
        return false;
    }
    name.rsplit_once('.')
        .is_some_and(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
}

/// Attachments nothing links to any more.
///
/// A note that loses its only picture leaves the file behind, and a vault that only ever grows is
/// one somebody eventually deletes wholesale. The sweep is deliberately *not* automatic on save:
/// the same picture can be linked from a note the user has not opened this year, so the set of
/// links is a whole-vault question and only a whole-vault pass may answer it.
pub fn unreferenced(paths: &Paths, linked: &[String]) -> Result<Vec<PathBuf>> {
    let dir = paths.attachments();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let kept: std::collections::HashSet<&str> = linked
        .iter()
        .filter_map(|link| link.rsplit('/').next())
        .collect();

    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))? {
        let entry = entry.map_err(|e| Error::io(&dir, e))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        // Only files this module named. A person's photograph is in here under whatever name they
        // gave it, and a sweep that deleted anything it did not recognise would take those with it.
        let recognised = locate(paths, &name).is_ok();
        if recognised && !kept.contains(name.as_ref()) {
            orphans.push(path);
        }
    }
    orphans.sort();
    Ok(orphans)
}

fn missing(name: &str) -> Error {
    Error::msg("attachment.not_found", format!("không có tệp {name}"))
}

/// Every attachment a piece of Markdown points at.
///
/// Text rather than a parsed document, because the same answer is wanted for a note, a meeting
/// summary and a transcript, and those are three shapes of one string.
#[must_use]
pub fn links_in(markdown: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = markdown;
    while let Some(at) = rest.find("attachments/") {
        rest = &rest[at..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == ')' || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        let link = &rest[..end];
        if is_attachment(link) {
            found.push(link.to_string());
        }
        rest = &rest[end.max(1)..];
    }
    found.sort();
    found.dedup();
    found
}

/// A path under the vault, for a caller that has a vault-relative link.
#[must_use]
pub fn path_of(paths: &Paths, link: &str) -> Option<PathBuf> {
    let name = link.strip_prefix("attachments/")?;
    locate(paths, name).ok().map(|(path, _)| path)
}

/// Whether a path is inside the attachments directory, for callers doing their own filing.
#[must_use]
pub fn contains(paths: &Paths, path: &Path) -> bool {
    path.starts_with(paths.attachments())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest thing each sniffer must accept: a real header, and nothing after it.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    const JPEG: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF";
    const WEBP_BYTES: &[u8] = b"RIFF\x24\x00\x00\x00WEBPVP8 ";

    fn vault() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        (tmp, paths)
    }

    #[test]
    fn a_picture_is_stored_under_a_name_made_of_its_own_bytes() {
        let (_tmp, paths) = vault();
        let link = store(&paths, PNG).unwrap();
        assert!(link.starts_with("attachments/"), "{link}");
        assert!(link.ends_with(".png"), "{link}");
        assert!(path_of(&paths, &link).unwrap().is_file());
    }

    /// Pasting one screenshot into a week of notes must cost one file.
    #[test]
    fn the_same_picture_twice_is_one_file() {
        let (_tmp, paths) = vault();
        let first = store(&paths, PNG).unwrap();
        let second = store(&paths, PNG).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read_dir(paths.attachments()).unwrap().count(), 1);
    }

    #[test]
    fn different_pictures_do_not_collide() {
        let (_tmp, paths) = vault();
        assert_ne!(store(&paths, PNG).unwrap(), store(&paths, JPEG).unwrap());
    }

    /// The interface is served from this daemon's origin, so an SVG served back under it is script
    /// running in the app. It is refused at the door rather than sniffed into a safe content type,
    /// because a file nobody recognises is not one to hand a browser.
    #[test]
    fn an_svg_is_refused_however_it_is_dressed_up() {
        let (_tmp, paths) = vault();
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        let err = store(&paths, svg).unwrap_err().to_string();
        assert!(err.contains("PNG"), "{err}");
        assert!(!paths.attachments().exists() || dir_is_empty(&paths));
    }

    #[test]
    fn html_wearing_a_png_extension_never_gets_that_far() {
        let (_tmp, paths) = vault();
        assert!(store(&paths, b"<html><script>alert(1)</script>").is_err());
    }

    #[test]
    fn a_webp_is_recognised_by_its_window_rather_than_its_prefix() {
        let (_tmp, paths) = vault();
        let link = store(&paths, WEBP_BYTES).unwrap();
        assert!(link.ends_with(".webp"), "{link}");
        // `RIFF` alone is a container, not a picture: a WAV file starts the same way.
        assert!(store(&paths, b"RIFF\x24\x00\x00\x00WAVEfmt ").is_err());
    }

    #[test]
    fn something_far_too_large_is_refused_before_it_is_written() {
        let (_tmp, paths) = vault();
        let mut huge = PNG.to_vec();
        huge.resize(MAX_BYTES + 1, 0);
        assert!(store(&paths, &huge).is_err());
        assert!(!paths.attachments().exists() || dir_is_empty(&paths));
    }

    /// This is reached from a URL, and it is the request somebody makes on the first day.
    #[test]
    fn a_name_that_walks_out_of_the_directory_is_not_a_name() {
        let (_tmp, paths) = vault();
        store(&paths, PNG).unwrap();
        for name in [
            "../../../etc/passwd",
            "..%2f..%2fetc%2fpasswd",
            "sub/dir.png",
            "/etc/hosts.png",
            "note.md",
        ] {
            assert!(locate(&paths, name).is_err(), "{name}");
        }
    }

    /// A person's photograph lives in the same folder under whatever name they gave it. Serving it
    /// is not this route's job, and neither is deleting it.
    #[test]
    fn a_file_this_module_did_not_name_is_neither_served_nor_swept() {
        let (_tmp, paths) = vault();
        std::fs::create_dir_all(paths.attachments()).unwrap();
        std::fs::write(paths.attachments().join("ngoc.jpg"), JPEG).unwrap();

        assert!(locate(&paths, "ngoc.jpg").is_err());
        assert!(unreferenced(&paths, &[]).unwrap().is_empty());
    }

    #[test]
    fn a_picture_nothing_links_to_is_reported_and_one_that_is_linked_is_not() {
        let (_tmp, paths) = vault();
        let kept = store(&paths, PNG).unwrap();
        let dropped = store(&paths, JPEG).unwrap();

        let orphans = unreferenced(&paths, std::slice::from_ref(&kept)).unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0], path_of(&paths, &dropped).unwrap());
    }

    #[test]
    fn links_are_read_out_of_the_markdown_a_note_is_made_of() {
        let markdown = "Xem ![sơ đồ](attachments/aaaa.png) và [tệp](attachments/bbbb.jpg).\n\
             Không phải attachments/sub/cc.png, cũng không phải attachments/.";
        assert_eq!(
            links_in(markdown),
            ["attachments/aaaa.png", "attachments/bbbb.jpg"]
        );
    }

    #[test]
    fn the_same_link_twice_is_counted_once() {
        let twice = "![a](attachments/aa.png) ![a](attachments/aa.png)";
        assert_eq!(links_in(twice), ["attachments/aa.png"]);
    }

    fn dir_is_empty(paths: &Paths) -> bool {
        std::fs::read_dir(paths.attachments())
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
    }
}
