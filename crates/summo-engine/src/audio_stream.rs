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

use summo_core::{Error, MeetingId, Result, segment::Lane, paths::Paths};

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
    let lane = match lane {
        "mic" => Lane::Mic,
        "system" => Lane::System,
        other => {
            return Err(Error::Other(format!(
                "no such lane `{other}`: expected `mic` or `system`"
            )));
        }
    };

    let dir = paths.audio_for(meeting);
    let path = dir.join(format!("{}.opus", lane_name(lane)));
    if !path.is_file() {
        return Err(Error::Other(format!(
            "no {} recording for meeting {meeting}",
            lane_name(lane)
        )));
    }
    Ok(path)
}

fn lane_name(lane: Lane) -> &'static str {
    match lane {
        Lane::Mic => "mic",
        Lane::System => "system",
    }
}

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
    file.read_exact(&mut buffer).map_err(|e| Error::io(path, e))?;
    Ok(buffer)
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
