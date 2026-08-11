//! Transcribing a file that was recorded somewhere else.
//!
//! `summo import` turns a Zoom recording into a 16 kHz mono WAV and stops there, which leaves the
//! user holding audio the app cannot read. This closes that gap by pushing the file through the
//! same [`SessionRunner`] a live meeting uses.
//!
//! Reusing the live pipeline rather than writing a batch one is deliberate. A separate offline path
//! would drift: it would segment differently, cluster speakers differently, and eventually produce
//! transcripts that do not match what the same audio would have produced live. Feeding the file in
//! at a fixed block size is the whole difference.
//!
//! Two things it does *not* do. It does not resample — the file must already be 16 kHz mono,
//! because guessing at a rate silently changes pitch and destroys word error rate, and
//! [`summo_media::Ffmpeg::to_wav`] already produces the right thing. And it does not report a
//! duration it has not read: progress comes from samples consumed, so a truncated file reports the
//! truncated length rather than the header's claim.

use std::path::Path;

use summo_core::{Error, Event, Result, segment::Lane};

use crate::runner::SessionRunner;

/// How much audio to hand the runner at once.
///
/// The framer re-slices this to the detector's exact width, so the only thing the block size
/// changes is how often progress is reported and how much memory the read holds. Two seconds is
/// small enough that a cancel feels immediate and large enough that the per-call overhead is noise.
pub const BLOCK: usize = 32_000;

/// The lane an imported file is transcribed on.
///
/// [`Lane::System`] rather than [`Lane::Mic`], because the mic lane is *defined* as the local user —
/// the pipeline skips speaker clustering on it for that reason. An imported recording has no such
/// guarantee: it is a room, a call, a podcast. Treating it as the microphone would collapse every
/// speaker in the file into one person.
pub const LANE: Lane = Lane::System;

/// How far through the file the transcription is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    /// Seconds of audio consumed so far.
    pub done_s: f64,
    /// Seconds of audio in the file, from its header.
    pub total_s: f64,
    /// Final segments produced so far.
    pub segments: usize,
}

impl Progress {
    /// A fraction in `0.0..=1.0`, or `None` when the file's length is unknown.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        (self.total_s > 0.0).then(|| (self.done_s / self.total_s).clamp(0.0, 1.0))
    }
}

/// A file's audio, in the shape the pipeline needs.
pub struct Wav {
    pub samples: Vec<f32>,
    pub rate: u32,
}

/// Prints the shape, never the samples: a minute of audio is a million floats, and one accidental
/// `{:?}` in a log line would be a megabyte of noise.
impl std::fmt::Debug for Wav {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wav")
            .field("samples", &self.samples.len())
            .field("rate", &self.rate)
            .finish()
    }
}

impl Wav {
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        if self.rate == 0 {
            return 0.0;
        }
        self.samples.len() as f64 / f64::from(self.rate)
    }
}

/// Read a mono 16 kHz WAV into float samples.
///
/// Anything else is refused rather than converted. A caller that has a 44.1 kHz stereo file should
/// go back through ffmpeg, which resamples properly; doing it here with a naive stride would be a
/// quiet quality regression that only shows up as a worse transcript.
pub fn read_wav(path: &Path) -> Result<Wav> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| Error::Other(format!("không đọc được {}: {e}", path.display())))?;
    let spec = reader.spec();

    if spec.channels != 1 {
        return Err(Error::msg(
            "audio.not_mono",
            format!(
                "{} có {} kênh; cần 1 kênh mono",
                path.display(),
                spec.channels
            ),
        ));
    }
    if spec.sample_rate != summo_media::TARGET_RATE {
        return Err(Error::Other(format!(
            "{} ở {} Hz; cần {} Hz",
            path.display(),
            spec.sample_rate,
            summo_media::TARGET_RATE
        )));
    }

    let samples = decode(reader)?;
    Ok(Wav {
        samples,
        rate: spec.sample_rate,
    })
}

fn decode(mut reader: hound::WavReader<std::io::BufReader<std::fs::File>>) -> Result<Vec<f32>> {
    let spec = reader.spec();
    let bad = |e: hound::Error| Error::Other(format!("file âm thanh hỏng: {e}"));

    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => {
            reader.samples::<f32>().map(|s| s.map_err(bad)).collect()
        }
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|v| f32::from(v) / 32_768.0).map_err(bad))
            .collect(),
        (hound::SampleFormat::Int, bits) => {
            // 24- and 32-bit integer PCM read as i32; the scale depends on the declared width, and
            // using the wrong one is a 256× gain error rather than a subtle one.
            let full = 2f32.powi(i32::from(bits) - 1);
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / full).map_err(bad))
                .collect()
        }
    }
}

/// Push a whole file through the pipeline, reporting progress as it goes.
///
/// The callback returning `false` stops the run and returns what has been produced so far — an
/// import of a two-hour file has to be cancellable, and abandoning the events would punish the user
/// for changing their mind.
pub fn transcribe(
    wav: &Wav,
    runner: &mut SessionRunner,
    mut on_progress: impl FnMut(Progress) -> bool,
) -> Result<Vec<Event>> {
    let total_s = wav.duration_s();
    let rate = f64::from(wav.rate.max(1));
    let mut events = Vec::new();
    let mut segments = 0usize;

    for (block, chunk) in wav.samples.chunks(BLOCK).enumerate() {
        let produced = runner.accept(LANE, chunk)?;
        segments += produced
            .iter()
            .filter(|e| matches!(e, Event::Final(_)))
            .count();
        events.extend(produced);

        let done_s = ((block + 1) * BLOCK).min(wav.samples.len()) as f64 / rate;
        if !on_progress(Progress {
            done_s,
            total_s,
            segments,
        }) {
            return Ok(events);
        }
    }

    // The last utterance is still open when the samples run out; without this the final sentence of
    // every imported file would be missing.
    events.extend(runner.flush()?);
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, spec: hound::WavSpec, samples: &[i16]) {
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    fn mono16() -> hound::WavSpec {
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        }
    }

    #[test]
    fn reads_the_format_ffmpeg_produces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        write(&path, mono16(), &[0, 16_384, -16_384, 32_767]);

        let wav = read_wav(&path).unwrap();
        assert_eq!(wav.rate, 16_000);
        assert_eq!(wav.samples.len(), 4);
        assert!((wav.samples[1] - 0.5).abs() < 1e-4);
        assert!((wav.samples[2] + 0.5).abs() < 1e-4);
    }

    /// Resampling here would be a silent quality loss, so the wrong rate has to be an error the
    /// caller sees rather than something this fixes up.
    #[test]
    fn the_wrong_sample_rate_is_refused_rather_than_resampled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        write(
            &path,
            hound::WavSpec {
                sample_rate: 44_100,
                ..mono16()
            },
            &[0, 1, 2],
        );

        let err = read_wav(&path).unwrap_err().to_string();
        assert!(err.contains("44100"), "{err}");
        assert!(err.contains("16000"), "{err}");
    }

    #[test]
    fn stereo_is_refused_rather_than_silently_halved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        write(
            &path,
            hound::WavSpec {
                channels: 2,
                ..mono16()
            },
            &[0, 1, 2, 3],
        );

        assert!(read_wav(&path).unwrap_err().to_string().contains("kênh"));
    }

    #[test]
    fn duration_comes_from_the_samples_that_were_actually_read() {
        let wav = Wav {
            samples: vec![0.0; 8_000],
            rate: 16_000,
        };
        assert!((wav.duration_s() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_zero_rate_does_not_divide_by_zero() {
        let wav = Wav {
            samples: vec![0.0; 10],
            rate: 0,
        };
        assert_eq!(wav.duration_s(), 0.0);
    }

    #[test]
    fn progress_past_the_end_still_reads_as_complete() {
        let p = Progress {
            done_s: 61.0,
            total_s: 60.0,
            segments: 3,
        };
        assert_eq!(p.fraction(), Some(1.0));
    }

    #[test]
    fn a_file_of_unknown_length_reports_no_fraction() {
        let p = Progress {
            done_s: 1.0,
            total_s: 0.0,
            segments: 0,
        };
        assert_eq!(p.fraction(), None);
    }
}
