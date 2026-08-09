//! Labelled audio for VAD evaluation.
//!
//! A dataset is a directory of 16 kHz mono WAV files, each with a sibling `.scv` file giving
//! ground-truth speech spans:
//!
//! ```text
//! clip-name,0.000,0.403,0,0.403,1.204,1,1.204,1.440,0,…
//!           └── start ┘ └ end ┘ └ 1 = speech, 0 = silence
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use summo_core::audio::SAMPLE_RATE;

/// A labelled interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub t0: f64,
    pub t1: f64,
    pub speech: bool,
}

/// One audio file plus its ground truth.
#[derive(Debug, Clone)]
pub struct Clip {
    pub name: String,
    pub path: PathBuf,
    /// Mono `f32` samples at 16 kHz.
    pub pcm: Vec<f32>,
    pub spans: Vec<Span>,
}

impl Clip {
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.pcm.len() as f64 / f64::from(SAMPLE_RATE)
    }

    /// Ground truth resampled onto a fixed frame grid, one bool per frame.
    ///
    /// A frame counts as speech when its midpoint falls inside a speech span, which avoids biasing
    /// the comparison toward either class at boundaries.
    #[must_use]
    pub fn frame_labels(&self, frame_len: usize) -> Vec<bool> {
        let frames = self.pcm.len() / frame_len;
        let frame_secs = frame_len as f64 / f64::from(SAMPLE_RATE);
        (0..frames)
            .map(|i| {
                let mid = (i as f64 + 0.5) * frame_secs;
                self.spans
                    .iter()
                    .find(|s| mid >= s.t0 && mid < s.t1)
                    .is_some_and(|s| s.speech)
            })
            .collect()
    }

    /// Times at which speech stops, i.e. the end of every speech span.
    ///
    /// These drive the release-latency metric, which is what actually determines how quickly the
    /// user sees final text.
    #[must_use]
    pub fn speech_offsets(&self) -> Vec<f64> {
        self.spans
            .iter()
            .filter(|s| s.speech)
            .map(|s| s.t1)
            .collect()
    }

    /// Times at which speech starts.
    #[must_use]
    pub fn speech_onsets(&self) -> Vec<f64> {
        self.spans
            .iter()
            .filter(|s| s.speech)
            .map(|s| s.t0)
            .collect()
    }
}

/// Load every WAV in `dir` that has a matching `.scv` label file.
pub fn load_dataset(dir: impl AsRef<Path>) -> Result<Vec<Clip>> {
    let dir = dir.as_ref();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read dataset directory {}", dir.display()))?;

    let mut clips = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wav") {
            continue;
        }
        let labels = path.with_extension("scv");
        if !labels.is_file() {
            tracing::warn!(path = %path.display(), "skipping: no .scv label file");
            continue;
        }
        clips.push(load_clip(&path, &labels)?);
    }

    if clips.is_empty() {
        bail!("no labelled clips found in {}", dir.display());
    }
    clips.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(clips)
}

fn load_clip(wav: &Path, scv: &Path) -> Result<Clip> {
    let pcm = read_wav(wav)?;
    let spans = parse_labels(
        &std::fs::read_to_string(scv).with_context(|| format!("cannot read {}", scv.display()))?,
    )
    .with_context(|| format!("cannot parse {}", scv.display()))?;

    Ok(Clip {
        name: wav
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: wav.to_path_buf(),
        pcm,
        spans,
    })
}

/// Read a mono 16 kHz WAV into `f32`.
///
/// The rate is required rather than resampled: a benchmark that silently resamples would measure
/// the resampler as much as the detector.
pub fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let spec = reader.spec();

    if spec.channels != 1 {
        bail!(
            "{}: expected mono, got {} channels",
            path.display(),
            spec.channels
        );
    }
    if spec.sample_rate != SAMPLE_RATE {
        bail!(
            "{}: expected {} Hz, got {}",
            path.display(),
            SAMPLE_RATE,
            spec.sample_rate
        );
    }

    let pcm = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / f32::from(i16::MAX);
            reader
                .samples::<i16>()
                .map(|s| s.map(|v| f32::from(v) * scale))
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(pcm)
}

/// Parse one `.scv` line into spans.
fn parse_labels(body: &str) -> Result<Vec<Span>> {
    let line = body
        .lines()
        .find(|l| !l.trim().is_empty())
        .context("label file is empty")?;

    let fields: Vec<&str> = line.trim().split(',').collect();
    if fields.len() < 4 {
        bail!("expected `name,t0,t1,flag,…`, got {} fields", fields.len());
    }

    // Field 0 is the clip name; the rest are (t0, t1, flag) triples.
    let triples = &fields[1..];
    if !triples.len().is_multiple_of(3) {
        bail!(
            "label fields must come in (t0, t1, flag) triples, got {}",
            triples.len()
        );
    }

    let mut spans = Vec::with_capacity(triples.len() / 3);
    for chunk in triples.chunks_exact(3) {
        let t0: f64 = chunk[0].trim().parse().context("bad start time")?;
        let t1: f64 = chunk[1].trim().parse().context("bad end time")?;
        let flag: u8 = chunk[2].trim().parse().context("bad speech flag")?;
        if t1 < t0 {
            bail!("span ends ({t1}) before it starts ({t0})");
        }
        spans.push(Span {
            t0,
            t1,
            speech: flag == 1,
        });
    }
    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "clip-01,0.000,0.403,0,0.403,1.204,1,1.204,1.440,0";

    #[test]
    fn parses_alternating_spans() {
        let spans = parse_labels(SAMPLE).unwrap();
        assert_eq!(spans.len(), 3);
        assert!(!spans[0].speech);
        assert!(spans[1].speech);
        assert_eq!(spans[1].t0, 0.403);
        assert_eq!(spans[1].t1, 1.204);
    }

    #[test]
    fn rejects_truncated_triples() {
        assert!(parse_labels("clip-01,0.0,1.0").is_err());
        assert!(parse_labels("clip-01,0.0,1.0,1,2.0").is_err());
    }

    #[test]
    fn rejects_inverted_spans() {
        assert!(parse_labels("clip-01,1.0,0.5,1").is_err());
    }

    #[test]
    fn empty_label_file_is_an_error() {
        assert!(parse_labels("\n\n").is_err());
    }

    fn clip_from(spans: Vec<Span>, secs: f64) -> Clip {
        Clip {
            name: "t".into(),
            path: PathBuf::new(),
            pcm: vec![0.0; (secs * f64::from(SAMPLE_RATE)) as usize],
            spans,
        }
    }

    #[test]
    fn frame_labels_follow_the_spans() {
        let clip = clip_from(parse_labels(SAMPLE).unwrap(), 1.44);
        // 100 ms frames: 0.0–0.4 silence, 0.4–1.2 speech, 1.2–1.44 silence.
        let labels = clip.frame_labels(1600);
        assert_eq!(labels.len(), 14);
        assert!(!labels[0], "frame at 0.05 s should be silence");
        assert!(labels[5], "frame at 0.55 s should be speech");
        assert!(!labels[13], "frame at 1.35 s should be silence");
    }

    #[test]
    fn onsets_and_offsets_come_from_speech_spans_only() {
        let clip = clip_from(parse_labels(SAMPLE).unwrap(), 1.44);
        assert_eq!(clip.speech_onsets(), vec![0.403]);
        assert_eq!(clip.speech_offsets(), vec![1.204]);
    }

    #[test]
    fn audio_beyond_the_last_span_counts_as_silence() {
        // Labels stop at 1.44 s but the file runs to 3 s; unlabelled tail must not read as speech.
        let clip = clip_from(parse_labels(SAMPLE).unwrap(), 3.0);
        let labels = clip.frame_labels(1600);
        assert!(!labels[25], "unlabelled tail should default to silence");
    }
}
