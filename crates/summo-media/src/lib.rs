//! Getting audio out of whatever the user has.
//!
//! Summo records its own meetings, but people also arrive with a folder of `.mp4` from Zoom, a
//! voice memo, a podcast they want transcribed. All of that is the same problem: decode something,
//! resample it to 16 kHz mono, hand it to the recogniser.
//!
//! **In this process first, ffmpeg second.** [`builtin`] decodes mp3, m4a, mp4, mov, mkv, wav,
//! flac and Vorbis with Symphonia — which is to say every format a phone, a laptop or a
//! conferencing tool produces — and needs nothing installed. That matters more than it sounds:
//! the setup checklist used to carry a step telling somebody to go and fetch a program before the
//! app they had just downloaded would import a recording, and an app you download should run.
//!
//! ffmpeg stays for what is left: Opus inside WebM, AC-3, WMA, and whatever container something
//! invents next. When it is absent those files fail with a message naming it; everything else
//! works.
//!
//! **ffmpeg is a sidecar process, not a linked library.** Three reasons, in order of how much they
//! matter:
//!
//! * *Licensing.* A linked ffmpeg drags GPL/LGPL obligations into an AGPL binary that is also sold
//!   commercially. Invoking a program the user already has is the same relationship a text editor
//!   has with a compiler.
//! * *Blast radius.* A malformed file crashes a subprocess, not the daemon holding somebody's
//!   meeting.
//! * *Size.* Statically linking every codec doubles the installer for a feature most users never
//!   touch.
//!
//! The cost is that ffmpeg has to be found rather than assumed, which is why [`probe`] exists and
//! why every error says what to install.

pub mod builtin;

use std::path::{Path, PathBuf};
use std::process::Command;

use summo_core::{Error, Result};

/// What the recogniser wants, and therefore what everything is converted to.
pub const TARGET_RATE: u32 = 16_000;

/// Decode to 16 kHz mono WAV, in this process if the format allows and through ffmpeg if not.
///
/// The order is the point. Nothing needs installing for the common case, and the fallback is only
/// reached by a file the built-in decoder genuinely cannot read — at which point the error names
/// ffmpeg, because that is now the only thing that would help.
pub fn to_wav16(input: &Path, output: &Path) -> Result<()> {
    match builtin::to_wav(input, output) {
        Ok(()) => Ok(()),
        Err(builtin_error) => {
            tracing::debug!(%builtin_error, "built-in decode failed, trying ffmpeg");
            match probe() {
                Ok(tools) => tools.to_wav(input, output),
                // Both failed, and only one of the two messages helps. The built-in decoder's is
                // about this file; ffmpeg's is about the machine. A person who gave us a `.wma`
                // needs to be told to install something, so that is the one that surfaces —
                // with the first reason kept beside it, because "unsupported format" and "ffmpeg
                // is not here" are different problems and the log should not have to guess.
                Err(missing) => Err(Error::Other(format!(
                    "{builtin_error}; ffmpeg would read it and is not available: {missing}"
                ))),
            }
        }
    }
}

/// What is in a file, in this process if possible.
pub fn info_of(path: &Path) -> Result<MediaInfo> {
    match builtin::info(path) {
        Ok(info) => Ok(info),
        Err(builtin_error) => match probe() {
            Ok(tools) => tools.info(path),
            Err(missing) => Err(Error::Other(format!(
                "{builtin_error}; ffmpeg would read it and is not available: {missing}"
            ))),
        },
    }
}

/// Extensions worth offering in a file picker.
///
/// Not a whitelist for correctness — ffmpeg decides what it can decode, and this list only shapes
/// what the interface suggests. Rejecting a file ffmpeg could have read would be worse than trying
/// and reporting the failure.
pub const SUGGESTED: [&str; 10] = [
    "mp3", "m4a", "wav", "flac", "ogg", "opus", "mp4", "mkv", "webm", "mov",
];

/// Where ffmpeg is, and whether it is usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// First line of `ffmpeg -version`, for the settings screen and bug reports.
    pub version: String,
}

/// Find ffmpeg, preferring one the app shipped over one on the path.
///
/// `SUMMO_FFMPEG` overrides both, because a user with a codec-limited system build needs a way to
/// point at a fuller one without reinstalling anything.
pub fn probe() -> Result<Ffmpeg> {
    let ffmpeg = std::env::var_os("SUMMO_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    let ffprobe = std::env::var_os("SUMMO_FFPROBE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffprobe"));

    let output = Command::new(&ffmpeg)
        .arg("-version")
        .output()
        .map_err(|e| {
            Error::Other(format!(
                "cannot run ffmpeg ({e}). Cài ffmpeg rồi thử lại, hoặc đặt SUMMO_FFMPEG trỏ tới nó."
            ))
        })?;
    if !output.status.success() {
        return Err(Error::Other(
            "ffmpeg is installed but refused to report its version".into(),
        ));
    }

    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("ffmpeg")
        .trim()
        .to_string();

    Ok(Ffmpeg {
        ffmpeg,
        ffprobe,
        version,
    })
}

/// What a media file contains.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub duration_s: f64,
    pub has_audio: bool,
    pub has_video: bool,
    /// Sample rate of the first audio stream, when there is one.
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

impl Ffmpeg {
    /// Inspect a file without decoding it.
    pub fn info(&self, path: &Path) -> Result<MediaInfo> {
        let output = Command::new(&self.ffprobe)
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(path)
            .output()
            .map_err(|e| Error::Other(format!("cannot run ffprobe: {e}")))?;

        if !output.status.success() {
            return Err(Error::Other(format!(
                "không đọc được {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_probe(&String::from_utf8_lossy(&output.stdout))
    }

    /// Decode to 16 kHz mono WAV, which is what every recogniser here wants.
    ///
    /// Downmixes rather than picking a channel: a meeting recorded with one speaker per channel
    /// would otherwise lose half the conversation.
    pub fn to_wav(&self, input: &Path, output: &Path) -> Result<()> {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }

        let result = Command::new(&self.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
            .arg(input)
            .args([
                "-vn",
                "-ac",
                "1",
                "-ar",
                &TARGET_RATE.to_string(),
                "-acodec",
                "pcm_s16le",
                "-f",
                "wav",
            ])
            .arg(output)
            .output()
            .map_err(|e| Error::Other(format!("cannot run ffmpeg: {e}")))?;

        if !result.status.success() {
            return Err(Error::Other(format!(
                "không tách được âm thanh từ {}: {}",
                input.display(),
                String::from_utf8_lossy(&result.stderr).trim()
            )));
        }
        Ok(())
    }
}

/// Read the fields that matter out of `ffprobe -print_format json`.
///
/// Split out so it can be tested against real ffprobe output without running ffprobe: the parsing
/// is where the bugs are, not the invocation.
pub fn parse_probe(json: &str) -> Result<MediaInfo> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| Error::Other(format!("ffprobe returned something unreadable: {e}")))?;

    let streams = value
        .get("streams")
        .and_then(|s| s.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();

    let audio = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("audio"));
    let has_video = streams
        .iter()
        .any(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("video"));

    // Duration lives on the container, but a stream-only file (raw audio) has it per stream.
    let duration_s = value
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .or_else(|| {
            audio
                .and_then(|a| a.get("duration"))
                .and_then(|d| d.as_str())
                .and_then(|d| d.parse::<f64>().ok())
        })
        .unwrap_or(0.0);

    Ok(MediaInfo {
        duration_s,
        has_audio: audio.is_some(),
        has_video,
        // ffprobe reports these as strings, which is a classic place to write `as_u64` and get None.
        sample_rate: audio
            .and_then(|a| a.get("sample_rate"))
            .and_then(|r| r.as_str())
            .and_then(|r| r.parse().ok()),
        channels: audio
            .and_then(|a| a.get("channels"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|c| u32::try_from(c).ok()),
    })
}

/// A title for an imported file, from its name.
///
/// `2026-08-10 Weekly sync.mp4` becomes `Weekly sync`: the date is already frontmatter, and
/// repeating it in the title makes every list read twice.
#[must_use]
pub fn title_from(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Bản ghi");

    let cleaned = stem
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '-' || c == '_' || c == '.')
        .trim();

    // Stripping left nothing, so the name *was* the date. Keep it verbatim rather than turning
    // `2026-08-10` into `2026 08 10`, which reads like a mistake.
    if cleaned.is_empty() {
        return if stem.trim().is_empty() {
            "Bản ghi".to_string()
        } else {
            stem.trim().to_string()
        };
    }

    let spaced = cleaned.replace(['_', '-'], " ");
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "Bản ghi".to_string()
    } else {
        collapsed
    }
}

/// Whether a path looks like something worth trying.
#[must_use]
pub fn looks_importable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| SUGGESTED.contains(&e.as_str()))
}

/// Every importable file directly inside `dir`, sorted.
///
/// Not recursive: importing a folder should import the folder the user pointed at, not everything
/// beneath it. A stray `node_modules` full of sample audio is not a meeting archive.
pub fn importable_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .flatten()
    {
        let path = entry.path();
        if path.is_file() && looks_importable(&path) {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &str = r#"{
      "streams": [
        {"codec_type": "video", "width": 1920},
        {"codec_type": "audio", "sample_rate": "48000", "channels": 2}
      ],
      "format": {"duration": "2538.123456"}
    }"#;

    #[test]
    fn a_video_with_sound_reports_both() {
        let info = parse_probe(PROBE).expect("parse");
        assert!(info.has_audio && info.has_video);
        assert_eq!(info.sample_rate, Some(48_000));
        assert_eq!(info.channels, Some(2));
        assert!((info.duration_s - 2_538.123_456).abs() < 1e-6);
    }

    /// ffprobe reports sample_rate as a *string*, which is where `as_u64` silently returns None.
    #[test]
    fn a_string_sample_rate_is_still_a_number() {
        let info = parse_probe(r#"{"streams":[{"codec_type":"audio","sample_rate":"16000"}]}"#)
            .expect("parse");
        assert_eq!(info.sample_rate, Some(16_000));
    }

    #[test]
    fn a_file_with_no_audio_is_recognised_as_such() {
        let info =
            parse_probe(r#"{"streams":[{"codec_type":"video"}],"format":{}}"#).expect("parse");
        assert!(!info.has_audio);
        assert!(info.has_video);
    }

    #[test]
    fn duration_falls_back_to_the_stream_when_the_container_has_none() {
        let info =
            parse_probe(r#"{"streams":[{"codec_type":"audio","duration":"12.5"}],"format":{}}"#)
                .expect("parse");
        assert!((info.duration_s - 12.5).abs() < 1e-9);
    }

    #[test]
    fn a_file_with_no_duration_reports_zero_rather_than_failing() {
        let info = parse_probe(r#"{"streams":[{"codec_type":"audio"}]}"#).expect("parse");
        assert_eq!(info.duration_s, 0.0);
    }

    #[test]
    fn unreadable_probe_output_is_an_error() {
        assert!(parse_probe("not json").is_err());
    }

    #[test]
    fn a_title_drops_a_leading_date() {
        assert_eq!(
            title_from(Path::new("2026-08-10 Weekly sync.mp4")),
            "Weekly sync"
        );
        assert_eq!(title_from(Path::new("20260810_hop_tuan.m4a")), "hop tuan");
    }

    #[test]
    fn a_title_keeps_a_name_that_is_only_a_date() {
        // Stripping everything would leave nothing to read in the library.
        assert_eq!(title_from(Path::new("2026-08-10.mp4")), "2026-08-10");
    }

    #[test]
    fn a_title_survives_vietnamese_and_separators() {
        assert_eq!(
            title_from(Path::new("hop-ngan-sach_quy-4.mp4")),
            "hop ngan sach quy 4"
        );
        assert_eq!(title_from(Path::new("Họp ngân sách.mp4")), "Họp ngân sách");
    }

    #[test]
    fn a_file_with_no_usable_name_still_gets_a_title() {
        assert_eq!(title_from(Path::new("   .mp4")), "Bản ghi");
        // A dotfile has no extension as far as the filesystem is concerned, so the stem is `.mp4`.
        assert_eq!(title_from(Path::new(".mp4")), "mp4");
    }

    #[test]
    fn the_picker_suggests_audio_and_video_but_not_documents() {
        assert!(looks_importable(Path::new("a.mp4")));
        assert!(
            looks_importable(Path::new("a.M4A")),
            "case should not matter"
        );
        assert!(!looks_importable(Path::new("a.pdf")));
        assert!(!looks_importable(Path::new("a")));
    }

    #[test]
    fn importing_a_folder_takes_that_folder_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.mp4"), b"x").unwrap();
        std::fs::write(dir.path().join("a.m4a"), b"x").unwrap();
        std::fs::write(dir.path().join("notes.pdf"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested").join("c.mp3"), b"x").unwrap();

        let found = importable_in(dir.path()).expect("scan");
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.m4a", "b.mp4"], "sorted, flat, media only");
    }

    #[test]
    fn scanning_a_folder_that_is_not_there_is_an_error() {
        assert!(importable_in(Path::new("/nonexistent/nope")).is_err());
    }

    /// Runs the real binary. Ignored so a machine without ffmpeg still passes the suite.
    #[test]
    #[ignore = "needs ffmpeg installed"]
    fn ffmpeg_converts_a_real_file_to_the_shape_the_recogniser_wants() {
        let tools = probe().expect("ffmpeg");
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tone.wav");

        // A second of a sine wave at 44.1 kHz stereo — the shape a phone recording arrives in.
        let made = Command::new(&tools.ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1:sample_rate=44100",
                "-ac",
                "2",
            ])
            .arg(&source)
            .status()
            .expect("make a fixture");
        assert!(made.success());

        let info = tools.info(&source).expect("probe");
        assert_eq!(info.sample_rate, Some(44_100));
        assert_eq!(info.channels, Some(2));

        let out = dir.path().join("out.wav");
        tools.to_wav(&source, &out).expect("convert");

        let converted = tools.info(&out).expect("probe the result");
        assert_eq!(converted.sample_rate, Some(TARGET_RATE));
        assert_eq!(
            converted.channels,
            Some(1),
            "downmixed, not one channel picked"
        );
    }
}
