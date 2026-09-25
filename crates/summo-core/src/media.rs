//! What a file is, from its name.
//!
//! One table, in the crate everything depends on, because three layers need the same answer and
//! they are in crates that cannot see each other: the vault decides whether a meeting has something
//! to *watch*, the daemon sends a content type a browser will accept, and the interface draws a
//! `<video>` or an `<audio>`. Three copies of an extension list is three chances to disagree about
//! `.mkv`, and the symptom of disagreeing is a player that loads and shows a black rectangle.
//!
//! By name rather than by content on purpose. Sniffing a container means opening the file, and this
//! is asked about files that may be on a drive that is not plugged in — the question "would this be
//! a video" has an answer even when the bytes are unreachable.

use std::path::Path;

/// The MIME type a browser needs to decide whether it can play a file.
///
/// Unknown extensions get `application/octet-stream`, which a media element refuses rather than
/// misinterprets — the honest answer for a container nobody here recognises.
#[must_use]
pub fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}

/// Whether this is something with pictures in it.
#[must_use]
pub fn is_video(path: &Path) -> bool {
    mime_for(path).starts_with("video/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_container_is_recognised_whatever_case_it_is_written_in() {
        assert_eq!(mime_for(Path::new("a.MP4")), "video/mp4");
        assert_eq!(mime_for(Path::new("a.Mp3")), "audio/mpeg");
    }

    #[test]
    fn video_and_audio_are_told_apart() {
        assert!(is_video(Path::new("zoom.mp4")));
        assert!(is_video(Path::new("obs.mkv")));
        assert!(!is_video(Path::new("memo.m4a")));
        assert!(!is_video(Path::new("mic.opus")));
    }

    /// A player told `application/octet-stream` declines; one told `video/mp4` about a spreadsheet
    /// loads it and shows nothing, which is the failure that has no error message.
    #[test]
    fn something_unrecognised_is_not_guessed_at() {
        assert_eq!(mime_for(Path::new("notes.txt")), "application/octet-stream");
        assert_eq!(
            mime_for(Path::new("noextension")),
            "application/octet-stream"
        );
        assert!(!is_video(Path::new("notes.txt")));
    }
}
