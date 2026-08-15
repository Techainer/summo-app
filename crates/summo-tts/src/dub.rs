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
//! **Time-stretching keeps the pitch.** A line that has to run 15% fast to fit its slot is fitted
//! by overlapping and re-aligning windows of it, not by playing it faster — see [`stretch`]. It
//! used to be plain resampling, which raised the pitch by up to four semitones at the plan's fastest
//! speed and made a dubbed voice change register every time a line was tight.

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
        let stretched = stretch(&take.samples, slot.speed, rate);
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

/// A window long enough to hold a pitch period, short enough to sit inside one phone.
///
/// 30 ms is the usual choice and the reasons pull in both directions. A window has to span at least
/// two periods of the lowest voice it will meet — 75 Hz is 13 ms — or the alignment search below
/// has nothing periodic to lock onto and the output warbles. Much longer and a window covers two
/// different sounds, so re-aligning it smears the consonant at the join.
const WINDOW_S: f64 = 0.030;

/// How far the alignment search may move a window: ±10 ms, which is more than one period of any
/// voice. Searching further finds better correlations that belong to the *previous* syllable.
const SEARCH_S: f64 = 0.010;

/// Fit `samples` into `speed` times less time, without moving the pitch.
///
/// WSOLA — waveform-similarity overlap-add. The signal is cut into overlapping windows, and the
/// windows are laid back down closer together (to speed up) or further apart (to slow down) than
/// they were taken. That alone would put waveforms down out of phase with each other, which cancels
/// as much as it adds and sounds metallic; so before each window is taken, a short search moves it
/// by up to [`SEARCH_S`] to wherever it best continues what has already been written. The pitch is
/// whatever the waveform's own period says it is, and nothing here changes that period.
///
/// This replaced linear resampling, which is what the same function used to do: a line at
/// [`crate::plan::MAX_SPEED`] came out about four semitones high, so the dubbed voice rose in pitch
/// exactly on the lines that were hardest to follow anyway.
///
/// The output length is exact — `samples.len() / speed`, rounded — because [`assemble`] places the
/// next line by the clock and not by where this one happened to end.
///
/// `rate` is needed because every constant here is a duration: at 8 kHz a 30 ms window is 240
/// samples and at 48 kHz it is 1,440, and using one number for both gives the low rate a window
/// with no periods in it.
#[must_use]
pub fn stretch(samples: &[f32], speed: f64, rate: u32) -> Vec<f32> {
    if samples.is_empty() || !(speed.is_finite() && speed > 0.0) || (speed - 1.0).abs() < 1e-6 {
        return samples.to_vec();
    }

    let out_len = ((samples.len() as f64) / speed).round().max(1.0) as usize;

    // Even, because the synthesis hop is half of it and a Hann window at exactly half overlap sums
    // to one — which is what lets the overlap-add below be a re-timing rather than a tremolo.
    let window = ((WINDOW_S * f64::from(rate.max(1))) as usize).max(32) & !1;

    // Nothing to overlap. A take this short is a word or a click, and a click has no pitch to
    // preserve — resampling it is both cheaper and, at these speeds, inaudible.
    if samples.len() < window * 2 {
        return resample(samples, speed, out_len);
    }

    let synthesis_hop = window / 2;
    let analysis_hop = ((synthesis_hop as f64) * speed).round().max(1.0) as usize;
    let search = ((SEARCH_S * f64::from(rate)) as usize).clamp(1, synthesis_hop / 2);
    let taper = hann(window);

    // Summed with the weights that produced it, and divided at the end. The window sums to one in
    // the middle of the track by construction; at the two ends, and wherever the alignment search
    // has pulled two windows apart, it does not — and dividing by what was actually laid down is
    // what keeps a fade-out from appearing in the last 15 ms of every line.
    let mut out = vec![0.0f32; out_len + window];
    let mut laid = vec![0.0f32; out_len + window];

    let last_start = samples.len() - window;
    let mut at = 0usize;
    let mut writing = 0usize;

    while writing + window <= out.len() {
        for i in 0..window {
            out[writing + i] += samples[at + i] * taper[i];
            laid[writing + i] += taper[i];
        }

        // What the ear expects next: the input as it continues from the window just written.
        let expected_from = at + synthesis_hop;
        if expected_from + window > samples.len() {
            break;
        }
        let expected = &samples[expected_from..expected_from + window];

        // Where the next window would come from if the timeline were simply cut, and the small
        // neighbourhood around it that is allowed instead.
        let ideal = at + analysis_hop;
        if ideal > last_start {
            break;
        }
        at = best_match(samples, expected, ideal, search, last_start);
        writing += synthesis_hop;
    }

    for (sample, weight) in out.iter_mut().zip(&laid) {
        // Below this the window is only just opening and the sample is near silence anyway;
        // dividing by it would turn the tail of the taper into amplified noise.
        if *weight > 1e-3 {
            *sample /= *weight;
        }
    }
    out.truncate(out_len);
    out
}

/// The offset near `ideal` whose window best continues `expected`.
///
/// Normalised by the candidate's own energy, so the search prefers the window that has the same
/// *shape* rather than the one that is loudest — an unnormalised dot product walks towards the
/// nearest vowel and leaves the consonants doubled.
fn best_match(
    samples: &[f32],
    expected: &[f32],
    ideal: usize,
    search: usize,
    last_start: usize,
) -> usize {
    let window = expected.len();
    let from = ideal.saturating_sub(search);
    let to = (ideal + search).min(last_start);

    let mut best = ideal.min(last_start);
    let mut best_score = f32::NEG_INFINITY;
    for start in from..=to {
        let candidate = &samples[start..start + window];
        let mut dot = 0.0f32;
        let mut energy = 0.0f32;
        for (a, b) in expected.iter().zip(candidate) {
            dot += a * b;
            energy += b * b;
        }
        let score = dot / (energy.sqrt() + 1e-9);
        if score > best_score {
            best_score = score;
            best = start;
        }
    }
    best
}

/// A raised cosine. Two of them at half overlap add to exactly one.
fn hann(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let phase = std::f32::consts::TAU * (i as f32) / (len as f32);
            0.5 - 0.5 * phase.cos()
        })
        .collect()
}

/// Linear resampling: the old behaviour, kept for takes too short to overlap-add.
///
/// This does move the pitch. It is reached only by a take shorter than two windows — 60 ms, which
/// is less than a syllable — where there is no periodicity to protect and no time for a listener to
/// hear a register change in.
fn resample(samples: &[f32], speed: f64, out_len: usize) -> Vec<f32> {
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
        let samples = tone(220.0, 16_000, 16_000);
        assert_eq!(stretch(&samples, 2.0, 16_000).len(), 8_000);
        assert_eq!(stretch(&samples, 0.5, 16_000).len(), 32_000);
    }

    #[test]
    fn a_speed_of_one_returns_the_samples_untouched() {
        let samples = vec![0.1, 0.2, 0.3];
        assert_eq!(stretch(&samples, 1.0, 16_000), samples);
    }

    #[test]
    fn a_nonsense_speed_does_not_produce_a_zero_length_or_infinite_take() {
        let samples = vec![0.1, 0.2, 0.3];
        assert_eq!(stretch(&samples, 0.0, 16_000), samples);
        assert_eq!(stretch(&samples, f64::NAN, 16_000), samples);
        assert_eq!(stretch(&samples, f64::INFINITY, 16_000), samples);
    }

    #[test]
    fn stretching_nothing_produces_nothing() {
        assert!(stretch(&[], 1.5, 16_000).is_empty());
    }

    /// A sine at `hz`, which is the only input whose pitch can be measured without a pitch tracker.
    fn tone(hz: f32, rate: u32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| (std::f32::consts::TAU * hz * (i as f32) / (rate as f32)).sin() * 0.8)
            .collect()
    }

    /// Cycles per second, counted from upward zero crossings over the steady middle of a signal.
    ///
    /// The ends are skipped: the first and last window of an overlap-add are the two the taper has
    /// not finished with, and a half-amplitude edge crosses zero in the same places anyway — but a
    /// take that starts mid-cycle would bias the count over so short a stretch.
    fn frequency_of(samples: &[f32], rate: u32) -> f32 {
        let from = samples.len() / 4;
        let to = samples.len() * 3 / 4;
        let middle = &samples[from..to];
        let crossings = middle
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        (crossings as f32) * (rate as f32) / (middle.len() as f32)
    }

    /// The whole reason this is not linear resampling.
    ///
    /// At the plan's fastest speed, resampling raised a 220 Hz voice to 286 Hz — about four
    /// semitones, which is a different person. This asserts the pitch is the pitch that went in.
    #[test]
    fn a_line_fitted_to_its_slot_comes_back_at_the_pitch_it_went_in_at() {
        let rate = 16_000;
        let source = tone(220.0, rate, rate as usize);

        for speed in [crate::plan::MIN_SPEED, 1.15, crate::plan::MAX_SPEED] {
            let fitted = stretch(&source, speed, rate);
            let hz = frequency_of(&fitted, rate);
            assert!(
                (hz - 220.0).abs() < 8.0,
                "at {speed}× the pitch came back as {hz} Hz, not 220 Hz"
            );
        }
    }

    /// Overlap-add cancels where it does not align, and the symptom is a track that fades in and
    /// out at the window rate — audible as a tremolo before it is measurable as anything else.
    #[test]
    fn overlapping_the_windows_does_not_leave_a_tremolo_in_the_level() {
        let rate = 16_000;
        let source = tone(220.0, rate, rate as usize);
        let fitted = stretch(&source, 1.25, rate);

        // Every 10 ms of the steady middle, against the level that went in.
        let block = (rate / 100) as usize;
        let mut quietest = f32::INFINITY;
        let mut loudest = 0.0f32;
        for chunk in fitted[block * 10..fitted.len() - block * 10].chunks(block) {
            let peak = chunk.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            quietest = quietest.min(peak);
            loudest = loudest.max(peak);
        }
        assert!(quietest > 0.55, "a window dropped to {quietest}, from 0.8");
        assert!(
            loudest < 1.0,
            "a window summed to {loudest}, past full scale"
        );
    }

    /// A take shorter than two windows takes the resampling path, which must still be well-behaved.
    #[test]
    fn a_take_too_short_to_overlap_is_still_fitted_to_the_right_length() {
        let samples = vec![0.5f32; 200];
        assert_eq!(stretch(&samples, 1.25, 16_000).len(), 160);
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
