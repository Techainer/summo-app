//! VAD evaluation.
//!
//! Runs a backend frame-by-frame over labelled audio and measures both the things a classifier
//! benchmark usually reports (precision, recall) and the one that actually decides how the app
//! feels: **release latency**, the delay between speech stopping and the detector noticing. The
//! gate cannot close a segment until the detector releases, so that delay is added directly to the
//! time before the user sees final text.

use std::time::Instant;

use anyhow::Result;
use summo_core::audio::SAMPLE_RATE;
use summo_vad::Vad;

use crate::{
    dataset::Clip,
    report::{VadMetrics, percentile},
};

/// How far past a true boundary we keep looking for the detector to react before giving up.
const MATCH_WINDOW_S: f64 = 1.5;

/// Run `vad` over every clip and compute metrics.
pub fn evaluate(vad: &mut dyn Vad, clips: &[Clip], threshold: f32) -> Result<VadMetrics> {
    let frame_len = vad.frame_len();

    let (mut tp, mut fp, mut tn, mut fna) = (0_u64, 0_u64, 0_u64, 0_u64);
    let mut onsets = Vec::new();
    let mut releases = Vec::new();
    let mut compute = std::time::Duration::ZERO;
    let mut audio_secs = 0.0;
    let mut frames_total = 0;

    for clip in clips {
        vad.reset();

        let truth = clip.frame_labels(frame_len);
        let mut predicted = Vec::with_capacity(truth.len());

        for chunk in clip.pcm.chunks_exact(frame_len) {
            let start = Instant::now();
            let prob = vad.feed_frame(chunk)?;
            compute += start.elapsed();
            predicted.push(prob >= threshold);
        }

        // `frame_labels` and `chunks_exact` both drop the ragged tail, so these agree.
        for (&t, &p) in truth.iter().zip(&predicted) {
            match (t, p) {
                (true, true) => tp += 1,
                (false, true) => fp += 1,
                (false, false) => tn += 1,
                (true, false) => fna += 1,
            }
        }

        let frame_secs = frame_len as f64 / f64::from(SAMPLE_RATE);
        onsets.extend(reaction_delays(
            &clip.speech_onsets(),
            &predicted,
            frame_secs,
            true,
        ));
        releases.extend(reaction_delays(
            &clip.speech_offsets(),
            &predicted,
            frame_secs,
            false,
        ));

        audio_secs += clip.duration();
        frames_total += predicted.len();
    }

    let precision = ratio(tp, tp + fp);
    let recall = ratio(tp, tp + fna);
    let f1 = if precision + recall > 0.0 {
        2.0 * precision * recall / (precision + recall)
    } else {
        0.0
    };

    let compute_ms = compute.as_secs_f64() * 1000.0;
    Ok(VadMetrics {
        precision,
        recall,
        f1,
        false_trigger_rate: ratio(fp, fp + tn),
        onset_ms_p50: percentile(&mut onsets.clone(), 50.0),
        release_ms_p50: percentile(&mut releases.clone(), 50.0),
        release_ms_p95: percentile(&mut releases, 95.0),
        rtf: if audio_secs > 0.0 {
            compute.as_secs_f64() / audio_secs
        } else {
            0.0
        },
        compute_ms,
        audio_secs,
        frames: frames_total,
    })
}

/// Delay in milliseconds between each ground-truth boundary and the detector reacting.
///
/// `want_speech` selects the direction: `true` looks for the first predicted-speech frame at or
/// after a speech onset, `false` for the first predicted-silence frame at or after a speech offset.
/// Boundaries the detector never reacts to within [`MATCH_WINDOW_S`] are excluded rather than
/// counted as zero — they are missed detections, already penalised by recall, and folding them in
/// as "instant" would flatter a detector that simply failed.
fn reaction_delays(
    boundaries: &[f64],
    predicted: &[bool],
    frame_secs: f64,
    want_speech: bool,
) -> Vec<f64> {
    let max_frames = (MATCH_WINDOW_S / frame_secs).ceil() as usize;
    let mut out = Vec::new();

    for &boundary in boundaries {
        let start = (boundary / frame_secs).floor() as usize;
        let end = (start + max_frames).min(predicted.len());
        if start >= predicted.len() {
            continue;
        }
        if let Some(offset) = (start..end).find(|&i| predicted[i] == want_speech) {
            let reacted_at = offset as f64 * frame_secs;
            out.push(((reacted_at - boundary).max(0.0)) * 1000.0);
        }
    }
    out
}

fn ratio(num: u64, den: u64) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Span;
    use summo_core::Result as CoreResult;

    /// A detector scripted from a fixed probability sequence, so metric maths can be checked
    /// without a model.
    struct ScriptedVad {
        probs: Vec<f32>,
        idx: usize,
        frame_len: usize,
    }

    impl Vad for ScriptedVad {
        fn frame_len(&self) -> usize {
            self.frame_len
        }
        fn feed_frame(&mut self, _frame: &[f32]) -> CoreResult<f32> {
            let p = self.probs.get(self.idx).copied().unwrap_or(0.0);
            self.idx += 1;
            Ok(p)
        }
        fn reset(&mut self) {
            self.idx = 0;
        }
        fn name(&self) -> &'static str {
            "scripted"
        }
    }

    /// One clip of `secs` seconds whose speech span is `[t0, t1)`.
    fn clip(secs: f64, t0: f64, t1: f64) -> Clip {
        Clip {
            name: "t".into(),
            path: std::path::PathBuf::new(),
            pcm: vec![0.0; (secs * f64::from(SAMPLE_RATE)) as usize],
            spans: vec![
                Span {
                    t0: 0.0,
                    t1: t0,
                    speech: false,
                },
                Span {
                    t0,
                    t1,
                    speech: true,
                },
                Span {
                    t0: t1,
                    t1: secs,
                    speech: false,
                },
            ],
        }
    }

    #[test]
    fn a_perfect_detector_scores_one() {
        // 100 ms frames over 1 s; speech from 0.3 to 0.7.
        let clip = clip(1.0, 0.3, 0.7);
        let probs = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0];
        let mut vad = ScriptedVad {
            probs,
            idx: 0,
            frame_len: 1600,
        };

        let m = evaluate(&mut vad, std::slice::from_ref(&clip), 0.5).unwrap();

        assert!((m.f1 - 1.0).abs() < 1e-9, "f1 was {}", m.f1);
        assert_eq!(m.false_trigger_rate, 0.0);
        // Reaction delays are derived from frame index × frame duration, so an exact hit lands
        // within floating-point noise of zero rather than exactly on it.
        assert!(m.onset_ms_p50 < 1e-6, "onset was {}", m.onset_ms_p50);
        assert!(m.release_ms_p50 < 1e-6, "release was {}", m.release_ms_p50);
    }

    #[test]
    fn release_latency_is_measured_from_the_speech_offset() {
        // Detector holds speech for two extra frames (200 ms) after speech really stops.
        let clip = clip(1.0, 0.3, 0.7);
        let probs = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0];
        let mut vad = ScriptedVad {
            probs,
            idx: 0,
            frame_len: 1600,
        };

        let m = evaluate(&mut vad, std::slice::from_ref(&clip), 0.5).unwrap();

        assert!(
            (m.release_ms_p50 - 200.0).abs() < 1.0,
            "expected ~200 ms release, got {}",
            m.release_ms_p50
        );
        assert!(m.recall > 0.99, "holding on longer must not cost recall");
        assert!(m.precision < 1.0, "the extra frames are false positives");
    }

    #[test]
    fn a_detector_that_never_fires_scores_zero_without_dividing_by_zero() {
        let clip = clip(1.0, 0.3, 0.7);
        let mut vad = ScriptedVad {
            probs: vec![0.0; 10],
            idx: 0,
            frame_len: 1600,
        };

        let m = evaluate(&mut vad, std::slice::from_ref(&clip), 0.5).unwrap();

        assert_eq!(m.precision, 0.0);
        assert_eq!(m.recall, 0.0);
        assert_eq!(m.f1, 0.0);
        assert_eq!(
            m.onset_ms_p50, 0.0,
            "unmatched boundaries are excluded, not counted as zero"
        );
    }

    #[test]
    fn false_triggers_are_measured_against_silence_only() {
        // Speech is detected correctly, plus one spurious frame during silence.
        let clip = clip(1.0, 0.3, 0.7);
        let probs = vec![0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0];
        let mut vad = ScriptedVad {
            probs,
            idx: 0,
            frame_len: 1600,
        };

        let m = evaluate(&mut vad, std::slice::from_ref(&clip), 0.5).unwrap();

        // Six silence frames, one of them wrongly called speech.
        assert!(
            (m.false_trigger_rate - 1.0 / 6.0).abs() < 1e-9,
            "got {}",
            m.false_trigger_rate
        );
    }

    #[test]
    fn state_is_reset_between_clips() {
        let clips = vec![clip(1.0, 0.3, 0.7), clip(1.0, 0.3, 0.7)];
        let probs = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0];
        let mut vad = ScriptedVad {
            probs,
            idx: 0,
            frame_len: 1600,
        };

        let m = evaluate(&mut vad, &clips, 0.5).unwrap();

        // Without a reset the second clip would read past the end of the script and score zero.
        assert!((m.f1 - 1.0).abs() < 1e-9, "f1 was {}", m.f1);
        assert_eq!(m.frames, 20);
    }

    #[test]
    fn rtf_and_audio_duration_are_accumulated() {
        let clips = vec![clip(1.0, 0.3, 0.7), clip(2.0, 0.5, 1.5)];
        let mut vad = ScriptedVad {
            probs: vec![1.0; 30],
            idx: 0,
            frame_len: 1600,
        };

        let m = evaluate(&mut vad, &clips, 0.5).unwrap();

        assert!((m.audio_secs - 3.0).abs() < 1e-6);
        assert!(m.rtf >= 0.0 && m.rtf < 1.0);
    }
}
