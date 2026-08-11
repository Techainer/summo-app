//! Assembling the dubbed track and writing it out.
//!
//! Once [`crate::plan`] has decided where each line goes, this puts the audio there. Three things
//! it does that are not obvious:
//!
//! **The original is kept underneath, quietly.** A dub that replaces the source outright loses
//! laughter, the sound of a door, and the fact that two people talked over each other — and it
//! removes the listener's only way to tell a mistranslation from a mishearing. Broadcast dubbing
//! keeps the source bed low for exactly this reason. [`Mix::under_gain`] is that bed.
//!
//! **Nothing is allowed to clip.** Two lines that overlap after a stretch will sum past 1.0, and a
//! float sample past 1.0 becomes a click the moment it is written to 16-bit. Peaks are found first
//! and the whole track scaled once, rather than each sample clamped — clamping is distortion,
//! scaling is a volume change.
//!
//! **Time-stretching here is resampling, not pitch-preserving.** Playing a line 15% fast raises its
//! pitch by about two semitones, which is audible but not comical, and it is honest about being a
//! placeholder: a proper phase-vocoder belongs in a DSP crate, and the plan already caps the speed
//! at the point where the artefact would start to matter. Recorded in [`stretch`] so nobody
//! mistakes it for finished work.

use std::path::Path;

use summo_core::{Error, Result};

use crate::plan::Plan;

/// How the dubbed voice and the original sit together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mix {
    /// Gain applied to the original recording under the dub. 0.0 removes it entirely.
    pub under_gain: f32,
    /// Gain applied to the synthesised voice.
    pub voice_gain: f32,
}

impl Default for Mix {
    fn default() -> Self {
        Self {
            // Audible enough to carry the room and to let a listener check the dub against the
            // original, low enough not to compete with it.
            under_gain: 0.18,
            voice_gain: 1.0,
        }
    }
}

/// One synthesised line, ready to be placed.
pub struct Take {
    pub seq: u64,
    pub samples: Vec<f32>,
}

/// Lay every take into a track of `total_s` at `rate`.
///
/// Takes with no matching slot are skipped rather than appended: a take whose line was removed from
/// the plan has no honest place in the timeline, and putting it at the end would be worse than
/// leaving it out.
#[must_use]
pub fn assemble(plan: &Plan, takes: &[Take], under: &[f32], rate: u32, mix: Mix) -> Vec<f32> {
    let length = under.len().max(track_len(plan, rate));
    let mut track = vec![0.0f32; length];

    for (i, sample) in under.iter().enumerate() {
        track[i] = sample * mix.under_gain;
    }

    for take in takes {
        let Some(slot) = plan.slots.iter().find(|s| s.seq == take.seq) else {
            continue;
        };
        let stretched = stretch(&take.samples, slot.speed);
        let start = (slot.at_s.max(0.0) * f64::from(rate)) as usize;

        for (i, sample) in stretched.iter().enumerate() {
            let Some(cell) = track.get_mut(start + i) else {
                // A line running past the end of the recording is truncated rather than growing the
                // track: the video it is being muxed against does not get longer.
                break;
            };
            *cell += sample * mix.voice_gain;
        }
    }

    normalize(&mut track);
    track
}

/// Samples needed to hold everything the plan places.
fn track_len(plan: &Plan, rate: u32) -> usize {
    plan.slots
        .iter()
        .map(|s| ((s.at_s + s.length_s) * f64::from(rate)).ceil().max(0.0) as usize)
        .max()
        .unwrap_or(0)
}

/// Resample `samples` to play `speed` times faster.
///
/// Linear interpolation, and it changes pitch — this is a placeholder for a phase vocoder, not a
/// substitute for one. It is tolerable because [`crate::plan::MAX_SPEED`] caps the change at 1.3×,
/// about four semitones, and because the alternative — leaving lines unfitted — is worse.
#[must_use]
pub fn stretch(samples: &[f32], speed: f64) -> Vec<f32> {
    if samples.is_empty() || !(speed.is_finite() && speed > 0.0) || (speed - 1.0).abs() < 1e-6 {
        return samples.to_vec();
    }

    let out_len = ((samples.len() as f64) / speed).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let position = i as f64 * speed;
        let index = position.floor() as usize;
        let frac = (position - index as f64) as f32;
        let a = samples.get(index).copied().unwrap_or(0.0);
        let b = samples.get(index + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Scale the whole track down if anything would clip.
///
/// One scale factor for the track, not per sample: clamping each sample is distortion that sounds
/// like crackle, while scaling is a volume change nobody notices. Quiet tracks are left alone —
/// raising them would amplify room noise between lines.
pub fn normalize(track: &mut [f32]) {
    let peak = track.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak <= 1.0 || peak == 0.0 {
        return;
    }
    // A hair under 1.0: rounding to 16-bit at exactly full scale wraps on some encoders.
    let scale = 0.999 / peak;
    for sample in track.iter_mut() {
        *sample *= scale;
    }
}

/// Write a mono track as a 16-bit WAV, which is what ffmpeg wants for the mux.
pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| Error::Other(format!("không ghi được {}: {e}", path.display())))?;
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        writer
            .write_sample((clamped * 32_767.0) as i16)
            .map_err(|e| Error::Other(format!("không ghi được {}: {e}", path.display())))?;
    }
    writer
        .finalize()
        .map_err(|e| Error::Other(format!("không đóng được {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Fit, Slot};

    fn slot(seq: u64, at_s: f64, speed: f64, length_s: f64) -> Slot {
        Slot {
            seq,
            at_s,
            room_s: length_s,
            speed,
            length_s,
            fit: Fit::Natural,
            over_s: 0.0,
        }
    }

    fn plan_of(slots: Vec<Slot>) -> Plan {
        Plan {
            slots,
            overflows: 0,
            worst_over_s: 0.0,
        }
    }

    #[test]
    fn a_take_lands_at_the_time_its_slot_names() {
        let plan = plan_of(vec![slot(1, 1.0, 1.0, 0.5)]);
        let take = Take {
            seq: 1,
            samples: vec![0.5; 4_000],
        };
        let track = assemble(
            &plan,
            &[take],
            &[],
            8_000,
            Mix {
                under_gain: 0.0,
                voice_gain: 1.0,
            },
        );

        assert_eq!(track[7_999], 0.0, "nothing before the slot");
        assert!(track[8_100] > 0.4, "audio inside the slot");
    }

    /// A dub that replaces the source loses the room, the laughter, and the listener's only way to
    /// check a translation against what was actually said.
    #[test]
    fn the_original_stays_underneath_at_low_gain() {
        let plan = plan_of(vec![]);
        let under = vec![1.0f32; 100];
        let track = assemble(&plan, &[], &under, 8_000, Mix::default());
        assert!((track[0] - Mix::default().under_gain).abs() < 1e-6);
    }

    #[test]
    fn under_gain_of_zero_removes_the_original_entirely() {
        let track = assemble(
            &plan_of(vec![]),
            &[],
            &vec![1.0f32; 100],
            8_000,
            Mix {
                under_gain: 0.0,
                voice_gain: 1.0,
            },
        );
        assert!(track.iter().all(|s| *s == 0.0));
    }

    /// Two overlapping lines sum past 1.0, and a float past 1.0 becomes a click in 16-bit.
    #[test]
    fn overlapping_lines_are_scaled_down_rather_than_clipped() {
        let plan = plan_of(vec![slot(1, 0.0, 1.0, 1.0), slot(2, 0.0, 1.0, 1.0)]);
        let takes = vec![
            Take {
                seq: 1,
                samples: vec![0.8; 800],
            },
            Take {
                seq: 2,
                samples: vec![0.8; 800],
            },
        ];
        let track = assemble(
            &plan,
            &takes,
            &[],
            800,
            Mix {
                under_gain: 0.0,
                voice_gain: 1.0,
            },
        );

        let peak = track.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= 1.0, "peak {peak}");
        // Scaled, not clamped: the shape is preserved, so every sample is the same value.
        assert!(track.iter().take(800).all(|s| (*s - track[0]).abs() < 1e-6));
    }

    #[test]
    fn a_quiet_track_is_not_pumped_up_to_full_scale() {
        let mut track = vec![0.1f32; 10];
        normalize(&mut track);
        assert!((track[0] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn silence_normalizes_without_dividing_by_zero() {
        let mut track = vec![0.0f32; 10];
        normalize(&mut track);
        assert!(track.iter().all(|s| *s == 0.0));
    }

    /// A take whose line left the plan has no honest place in the timeline; appending it at the end
    /// would put words where nobody spoke.
    #[test]
    fn a_take_with_no_slot_is_dropped_rather_than_appended() {
        let plan = plan_of(vec![slot(1, 0.0, 1.0, 0.1)]);
        let takes = vec![Take {
            seq: 99,
            samples: vec![1.0; 800],
        }];
        let track = assemble(
            &plan,
            &takes,
            &[],
            800,
            Mix {
                under_gain: 0.0,
                voice_gain: 1.0,
            },
        );
        assert!(track.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_line_running_past_the_end_is_truncated_not_grown_onto_the_video() {
        let plan = plan_of(vec![slot(1, 0.0, 1.0, 0.5)]);
        let under = vec![0.0f32; 400];
        let takes = vec![Take {
            seq: 1,
            samples: vec![1.0; 4_000],
        }];
        let track = assemble(
            &plan,
            &takes,
            &under,
            800,
            Mix {
                under_gain: 0.0,
                voice_gain: 1.0,
            },
        );
        assert_eq!(track.len(), 400.max((0.5 * 800.0) as usize));
    }

    #[test]
    fn speeding_a_line_up_makes_it_shorter_in_proportion() {
        let samples = vec![0.5f32; 1_000];
        assert_eq!(stretch(&samples, 2.0).len(), 500);
        assert_eq!(stretch(&samples, 0.5).len(), 2_000);
    }

    #[test]
    fn a_speed_of_one_returns_the_samples_untouched() {
        let samples = vec![0.1, 0.2, 0.3];
        assert_eq!(stretch(&samples, 1.0), samples);
    }

    #[test]
    fn a_nonsense_speed_does_not_produce_a_zero_length_or_infinite_take() {
        let samples = vec![0.1, 0.2, 0.3];
        assert_eq!(stretch(&samples, 0.0), samples);
        assert_eq!(stretch(&samples, f64::NAN), samples);
        assert_eq!(stretch(&samples, f64::INFINITY), samples);
    }

    #[test]
    fn stretching_nothing_produces_nothing() {
        assert!(stretch(&[], 1.5).is_empty());
    }

    #[test]
    fn a_written_wav_reads_back_at_the_rate_it_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out").join("dub.wav");
        write_wav(&path, &[0.0, 0.5, -0.5], 48_000).unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.len(), 3);
    }
}
