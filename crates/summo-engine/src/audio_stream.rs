//! Serving a meeting's recording back to the player.
//!
//! An hour of Opus is about 5 MB per lane, which is small enough to send whole — but a player that
//! cannot seek until the whole file has arrived is not a player. So this answers `Range` requests,
//! which is what makes the scrubber work: the browser asks for the bytes around wherever the user
//! clicked and starts decoding from there.
//!
//! Two things this refuses to do, both for the same reason — the lane name arrives in a URL:
//!
//! * It never joins a caller-supplied string into a path. Lanes are an enum, and anything that is
//!   not `mic` or `system` is rejected before a path is built.
//! * It never serves a file outside the meeting's own directory.
//!
//! Without those, `GET /meetings/x/audio/../../../etc/passwd` is a file read, and the daemon holds
//! a bearer token precisely so that a page which gets hold of it cannot do that.

use std::path::{Path, PathBuf};

use summo_core::{Error, MeetingId, Result, paths::Paths, segment::Lane};

/// A byte range to send, resolved against the file's real length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u64,
    /// Inclusive, as HTTP counts.
    pub end: u64,
    pub total: u64,
}

impl Span {
    #[must_use]
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `Content-Range` header value for a partial response.
    #[must_use]
    pub fn content_range(&self) -> String {
        format!("bytes {}-{}/{}", self.start, self.end, self.total)
    }
}

/// Resolve a lane name from a URL into the file on disk.
///
/// Rejects anything that is not a known lane, so no caller-supplied text ever reaches a path.
pub fn locate(paths: &Paths, meeting: &MeetingId, lane: &str) -> Result<PathBuf> {
    // `import` is not a `Lane` — nothing was captured, a file was decoded — but it is a track the
    // player has to be able to fetch, and it is the only one an imported meeting has.
    //
    // It was missing, and the consequence was quiet and total: the detail view lists the audio
    // directory, the interface turns every name in it into a lane, and an import has exactly one
    // file in there. So every imported meeting drew a player whose only track answered
    // `no such lane 'import'` — recorded meetings played back, imported ones never had.
    let file = match lane {
        "mic" => format!("{}.opus", lane_name(Lane::Mic)),
        "system" => format!("{}.opus", lane_name(Lane::System)),
        "import" => "import.wav".to_string(),
        other => {
            return Err(Error::Other(format!(
                "no such lane `{other}`: expected `mic`, `system` or `import`"
            )));
        }
    };

    let path = paths.audio_for(meeting).join(&file);
    if !path.is_file() {
        return Err(Error::Other(format!(
            "no {lane} recording for meeting {meeting}"
        )));
    }
    Ok(path)
}

/// The content type for a track served by [`locate`].
///
/// A recorded lane is Opus in Ogg; an import is the 16 kHz wav the recogniser was fed. Sending
/// `audio/ogg` for a wav is the kind of wrong that works in one browser and not the next.
#[must_use]
pub fn lane_mime(lane: &str) -> &'static str {
    match lane {
        "import" => "audio/wav",
        _ => "audio/ogg",
    }
}

fn lane_name(lane: Lane) -> &'static str {
    match lane {
        Lane::Mic => "mic",
        Lane::System => "system",
    }
}

/// The file name a kept copy of an imported source is stored under.
///
/// The extension is appended, because a browser decides what it can play from the MIME type and
/// this is where that type comes from. The stem is fixed so nothing a user typed becomes a path.
pub const KEPT_SOURCE_STEM: &str = "source";

/// Where a kept copy of the imported media would be, extension and all.
///
/// A directory listing rather than a stored name: the extension is the only part that varies, and
/// looking it up beats writing it down in two places that can disagree.
#[must_use]
pub fn kept_source(paths: &Paths, meeting: &MeetingId) -> Option<PathBuf> {
    std::fs::read_dir(paths.audio_for(meeting))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s == KEPT_SOURCE_STEM)
        })
}

/// What went wrong when the media a meeting names cannot be played.
///
/// An enum rather than a string because the three cases want three different things on screen: one
/// is "this meeting was recorded, there is nothing to watch", one is "the file is where it always
/// was", and one is "the file has moved and here is where it used to be". Collapsing them into
/// `404` is what turns a recoverable situation into a broken player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoSource {
    /// Recorded here rather than imported: there is no original, and that is not a fault.
    NotImported,
    /// The meeting names a file, and it is not there any more.
    Missing(String),
}

/// The media an imported meeting came from: the kept copy first, then where it came from.
///
/// The copy wins because it is the one that cannot move. The original is tried next rather than
/// ignored, so somebody who imported without keeping a copy still gets their video back as long as
/// the file is where they left it — and gets told *which* file when it is not, instead of a player
/// that silently will not start.
///
/// # Errors
///
/// [`NoSource`] when the meeting was recorded rather than imported, or when the file it names is
/// gone.
pub fn locate_source(
    paths: &Paths,
    meeting: &MeetingId,
    declared: Option<&str>,
) -> std::result::Result<PathBuf, NoSource> {
    if let Some(copy) = kept_source(paths, meeting) {
        return Ok(copy);
    }
    let declared = declared.map(str::trim).filter(|s| !s.is_empty());
    let Some(declared) = declared else {
        return Err(NoSource::NotImported);
    };
    let path = PathBuf::from(declared);
    if path.is_file() {
        return Ok(path);
    }
    Err(NoSource::Missing(declared.to_string()))
}

pub use summo_core::media::{is_video, mime_for};

/// Parse a `Range` header into the span to send.
///
/// Returns `Ok(None)` when there is no header or it asks for something this does not implement —
/// multi-range, for instance — because the correct answer then is the whole file, not an error.
/// Returns `Err` only when the header is well-formed but unsatisfiable, which is a `416`.
pub fn parse_range(header: Option<&str>, total: u64) -> Result<Option<Span>> {
    let Some(header) = header else {
        return Ok(None);
    };
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return Ok(None);
    };
    // Multi-range is legal and almost never used; sending the whole file is a valid response.
    if spec.contains(',') {
        return Ok(None);
    }
    let Some((from, to)) = spec.split_once('-') else {
        return Ok(None);
    };

    let (start, end) = match (from.trim(), to.trim()) {
        // `bytes=-500` — the last 500 bytes.
        ("", suffix) => {
            let len: u64 = suffix.parse().map_err(|_| unsatisfiable(spec))?;
            if len == 0 {
                return Err(unsatisfiable(spec));
            }
            (total.saturating_sub(len), total.saturating_sub(1))
        }
        // `bytes=500-` — from 500 to the end.
        (prefix, "") => {
            let start: u64 = prefix.parse().map_err(|_| unsatisfiable(spec))?;
            (start, total.saturating_sub(1))
        }
        (prefix, suffix) => {
            let start: u64 = prefix.parse().map_err(|_| unsatisfiable(spec))?;
            let end: u64 = suffix.parse().map_err(|_| unsatisfiable(spec))?;
            // A client may ask past the end; clamp rather than refuse.
            (start, end.min(total.saturating_sub(1)))
        }
    };

    if total == 0 || start >= total || start > end {
        return Err(unsatisfiable(spec));
    }
    Ok(Some(Span { start, end, total }))
}

fn unsatisfiable(spec: &str) -> Error {
    Error::Other(format!("range not satisfiable: {spec}"))
}

/// Read one span out of a file.
pub fn read_span(path: &Path, span: Span) -> Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    file.seek(SeekFrom::Start(span.start))
        .map_err(|e| Error::io(path, e))?;

    let mut buffer = vec![0u8; usize::try_from(span.len()).unwrap_or(usize::MAX)];
    file.read_exact(&mut buffer)
        .map_err(|e| Error::io(path, e))?;
    Ok(buffer)
}

#[cfg(test)]
mod import_lane_tests {
    use super::*;

    /// Every imported meeting drew a player whose only track was a 404.
    ///
    /// The detail view lists the meeting's audio directory and the interface makes a lane of each
    /// name in it. An import puts exactly one file there, `import.wav`, and this function knew only
    /// `mic` and `system` — so recorded meetings played back and imported ones never had.
    #[test]
    fn an_import_is_a_track_the_player_can_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        let meeting = MeetingId::from("m1".to_string());
        let dir = paths.audio_for(&meeting);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("import.wav"), b"RIFF").unwrap();

        let found = locate(&paths, &meeting, "import").expect("the import lane resolves");
        assert!(found.ends_with("import.wav"));
        assert_eq!(lane_mime("import"), "audio/wav");
        assert_eq!(lane_mime("mic"), "audio/ogg");
    }

    /// Adding a lane must not turn the lane name back into a path.
    #[test]
    fn nothing_else_resolves_to_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        let meeting = MeetingId::from("m1".to_string());
        for lane in ["../../etc/passwd", "source", "import.wav", ""] {
            assert!(locate(&paths, &meeting, lane).is_err(), "{lane} resolved");
        }
    }

    /// Three different situations, three different things to say.
    #[test]
    fn a_missing_source_says_which_kind_of_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::at(tmp.path());
        let meeting = MeetingId::from("m1".to_string());

        assert_eq!(
            locate_source(&paths, &meeting, None),
            Err(NoSource::NotImported)
        );
        assert_eq!(
            locate_source(&paths, &meeting, Some("/gone/holp.mp4")),
            Err(NoSource::Missing("/gone/holp.mp4".to_string()))
        );

        // A kept copy wins over the original, because it is the one that cannot move.
        let dir = paths.audio_for(&meeting);
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("source.mp4");
        std::fs::write(&kept, b"x").unwrap();
        assert_eq!(
            locate_source(&paths, &meeting, Some("/gone/holp.mp4")),
            Ok(kept)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_with_audio(lane: &str) -> (TempDir, Paths, MeetingId) {
        let dir = TempDir::new().unwrap();
        let paths = Paths::at(dir.path());
        let meeting = MeetingId::from("01A".to_string());
        let audio = paths.audio_for(&meeting);
        std::fs::create_dir_all(&audio).unwrap();
        std::fs::write(audio.join(format!("{lane}.opus")), vec![7u8; 1000]).unwrap();
        (dir, paths, meeting)
    }

    #[test]
    fn a_known_lane_resolves_to_its_file() {
        let (_d, paths, meeting) = paths_with_audio("mic");
        let path = locate(&paths, &meeting, "mic").expect("locate");
        assert!(path.ends_with("mic.opus"));
    }

    #[test]
    fn a_missing_recording_is_reported_rather_than_served_empty() {
        let (_d, paths, meeting) = paths_with_audio("mic");
        assert!(locate(&paths, &meeting, "system").is_err());
    }

    /// The reason lanes are an enum: the name arrives in a URL.
    #[test]
    fn a_traversal_attempt_never_reaches_a_path() {
        let (_d, paths, meeting) = paths_with_audio("mic");
        for lane in ["../../../etc/passwd", "..", "mic/../../x", "", "MIC"] {
            let err = locate(&paths, &meeting, lane).expect_err("must be refused");
            assert!(err.to_string().contains("no such lane"), "{lane}: {err}");
        }
    }

    #[test]
    fn no_range_header_means_the_whole_file() {
        assert_eq!(parse_range(None, 1000).unwrap(), None);
    }

    #[test]
    fn a_closed_range_is_parsed() {
        let span = parse_range(Some("bytes=100-199"), 1000).unwrap().unwrap();
        assert_eq!((span.start, span.end, span.len()), (100, 199, 100));
        assert_eq!(span.content_range(), "bytes 100-199/1000");
    }

    #[test]
    fn an_open_ended_range_runs_to_the_end() {
        let span = parse_range(Some("bytes=900-"), 1000).unwrap().unwrap();
        assert_eq!((span.start, span.end), (900, 999));
    }

    #[test]
    fn a_suffix_range_takes_the_last_bytes() {
        let span = parse_range(Some("bytes=-100"), 1000).unwrap().unwrap();
        assert_eq!((span.start, span.end), (900, 999));
    }

    /// Players routinely ask for more than exists; clamping beats refusing.
    #[test]
    fn a_range_past_the_end_is_clamped() {
        let span = parse_range(Some("bytes=900-99999"), 1000).unwrap().unwrap();
        assert_eq!((span.start, span.end), (900, 999));
    }

    #[test]
    fn a_range_starting_past_the_end_is_unsatisfiable() {
        assert!(parse_range(Some("bytes=1000-"), 1000).is_err());
        assert!(parse_range(Some("bytes=5000-6000"), 1000).is_err());
    }

    #[test]
    fn an_empty_file_satisfies_no_range() {
        assert!(parse_range(Some("bytes=0-10"), 0).is_err());
    }

    #[test]
    fn a_backwards_range_is_unsatisfiable() {
        assert!(parse_range(Some("bytes=500-100"), 1000).is_err());
    }

    #[test]
    fn nonsense_in_the_header_falls_back_to_the_whole_file() {
        // Not an error: an unparseable unit or form means "I do not implement that", and the
        // whole file is always a correct answer to a GET.
        assert_eq!(parse_range(Some("items=0-10"), 1000).unwrap(), None);
        assert_eq!(parse_range(Some("bytes=0-10, 20-30"), 1000).unwrap(), None);
        assert_eq!(parse_range(Some("garbage"), 1000).unwrap(), None);
    }

    #[test]
    fn a_non_numeric_bound_is_refused() {
        assert!(parse_range(Some("bytes=abc-def"), 1000).is_err());
    }

    #[test]
    fn reading_a_span_returns_exactly_those_bytes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.opus");
        std::fs::write(&path, (0..=255u8).collect::<Vec<_>>()).unwrap();

        let span = parse_range(Some("bytes=10-19"), 256).unwrap().unwrap();
        let bytes = read_span(&path, span).expect("read");
        assert_eq!(bytes, (10..=19u8).collect::<Vec<_>>());
    }
}
