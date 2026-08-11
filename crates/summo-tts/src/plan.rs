//! Deciding where each dubbed line goes, before anything is synthesised.
//!
//! The naive approach — synthesise, then squeeze whatever comes out into the slot it came from —
//! fails in a specific and ugly way. English is often 25% shorter than the Vietnamese it was
//! translated from, so a line stretched to fill its slot sounds drugged; Japanese is often longer,
//! so a line squeezed to fit sounds like a chipmunk. Do that for an hour and the result is
//! unwatchable even though every line is technically in the right place.
//!
//! So planning is separate from synthesis, and it makes three decisions explicitly.
//!
//! **How far a line may be stretched.** [`MIN_SPEED`]/[`MAX_SPEED`] bound it. Outside that range
//! the fit is [`Fit::Overflow`] and the line is allowed to run past its slot rather than be
//! mangled.
//!
//! **Where the room is.** A slot is not only the original utterance: the silence after it, up to
//! the next speaker, is fair game. Most overruns disappear once the gap is counted, and using it
//! costs nothing because nobody is talking.
//!
//! **What the drift is.** Every overflow pushes the following lines, and the plan reports the
//! worst case so the caller can say "this will drift 1.2 s by the end" instead of the user
//! discovering it in the last minute of a film.

use serde::Serialize;

/// Slowest a line may be played to fill its slot. Below this, speech sounds drawn out and drunk.
pub const MIN_SPEED: f64 = 0.85;

/// Fastest a line may be played to fit its slot.
///
/// Comfortably intelligible speech tops out around here; past 1.3 listeners report effort even when
/// they can still follow it, and this runs for the length of a meeting rather than a sentence.
pub const MAX_SPEED: f64 = 1.30;

/// Silence left after a line before the next one starts.
///
/// Without it, a line stretched into the following gap butts straight into the next speaker and the
/// two run together as one breath.
pub const GUARD_S: f64 = 0.08;

/// One utterance to dub.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The transcript segment this came from.
    pub seq: u64,
    pub text: String,
    pub t0: f64,
    pub t1: f64,
    /// How long the synthesiser actually produced for this text.
    pub spoken_s: f64,
}

/// How a line was fitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fit {
    /// It fits as-is; play at natural speed.
    Natural,
    /// Sped up or slowed down within the allowed range.
    Adjusted,
    /// It could not be made to fit without going outside the range, so it runs long.
    Overflow,
}

/// Where one line ends up.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Slot {
    pub seq: u64,
    /// When to start playing it.
    pub at_s: f64,
    /// Room available, including the usable part of the gap that follows.
    pub room_s: f64,
    /// Playback rate to apply: 1.0 is untouched, 1.2 is 20% faster.
    pub speed: f64,
    /// How long it will actually occupy after the speed change.
    pub length_s: f64,
    pub fit: Fit,
    /// How far past its room it runs. Zero unless [`Fit::Overflow`].
    pub over_s: f64,
}

/// A whole dub.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub slots: Vec<Slot>,
    /// Lines that could not be made to fit.
    pub overflows: usize,
    /// The worst single overrun, in seconds — what the caller warns about.
    pub worst_over_s: f64,
}

impl Plan {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.overflows == 0
    }
}

/// Work out where every line goes.
///
/// `lines` must be in time order; `total_s` is the length of the original recording, which decides
/// how much room the last line has. A `total_s` shorter than the last line's end is treated as that
/// end — a caller passing a stale duration should not silently lose the final sentence.
#[must_use]
pub fn plan(lines: &[Line], total_s: f64) -> Plan {
    let end_of_audio = lines.last().map_or(total_s, |last| total_s.max(last.t1));

    let mut slots = Vec::with_capacity(lines.len());
    let mut overflows = 0usize;
    let mut worst_over_s: f64 = 0.0;

    for (i, line) in lines.iter().enumerate() {
        // The gap up to the next speaker is usable; the guard keeps the two from touching.
        let next_start = lines.get(i + 1).map_or(end_of_audio, |next| next.t0);
        let room_s = (next_start - GUARD_S - line.t0)
            .max(line.t1 - line.t0)
            .max(0.0);

        let (speed, fit) = fit_into(line.spoken_s, room_s);
        let length_s = if speed > 0.0 {
            line.spoken_s / speed
        } else {
            line.spoken_s
        };
        let over_s = (length_s - room_s).max(0.0);

        if fit == Fit::Overflow {
            overflows += 1;
            worst_over_s = worst_over_s.max(over_s);
        }

        slots.push(Slot {
            seq: line.seq,
            at_s: line.t0,
            room_s,
            speed,
            length_s,
            fit,
            over_s,
        });
    }

    Plan {
        slots,
        overflows,
        worst_over_s,
    }
}

/// The speed to play `spoken_s` of speech at so it occupies `room_s`, and whether that worked.
///
/// Slowing down to fill a slot is deliberately *not* done. A line shorter than its slot is left
/// alone and the remainder is silence — which is what the original had too, roughly, and which
/// sounds like a pause rather than like a stretched vowel.
fn fit_into(spoken_s: f64, room_s: f64) -> (f64, Fit) {
    if spoken_s <= 0.0 || room_s <= 0.0 {
        return (1.0, Fit::Natural);
    }
    if spoken_s <= room_s {
        return (1.0, Fit::Natural);
    }

    let needed = spoken_s / room_s;
    if needed <= MAX_SPEED {
        (needed, Fit::Adjusted)
    } else {
        // Capped rather than left at 1.0: running 30% fast and overflowing by a little beats
        // running at natural speed and overflowing by a lot.
        (MAX_SPEED, Fit::Overflow)
    }
}

/// How much a line would have to be slowed to fill its slot, for a caller that wants to.
///
/// Exposed rather than applied, because filling silence is a taste decision — a documentary dub
/// wants it, a meeting recording does not.
#[must_use]
pub fn stretch_to_fill(spoken_s: f64, room_s: f64) -> f64 {
    if spoken_s <= 0.0 || room_s <= 0.0 {
        return 1.0;
    }
    (spoken_s / room_s).clamp(MIN_SPEED, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seq: u64, t0: f64, t1: f64, spoken_s: f64) -> Line {
        Line {
            seq,
            text: format!("line {seq}"),
            t0,
            t1,
            spoken_s,
        }
    }

    #[test]
    fn a_line_that_already_fits_is_left_alone() {
        let plan = plan(&[line(1, 0.0, 5.0, 3.0)], 5.0);
        assert_eq!(plan.slots[0].speed, 1.0);
        assert_eq!(plan.slots[0].fit, Fit::Natural);
        assert!(plan.is_clean());
    }

    /// The whole reason planning is separate: a line that overruns its own utterance usually fits
    /// once the silence before the next speaker is counted, and that silence is free.
    #[test]
    fn the_gap_before_the_next_speaker_counts_as_room() {
        // Speaks for 2.0s, its own utterance is 1.0s, but nobody talks until 4.0s.
        let lines = [line(1, 0.0, 1.0, 2.0), line(2, 4.0, 5.0, 0.5)];
        let plan = plan(&lines, 5.0);
        assert_eq!(plan.slots[0].fit, Fit::Natural, "{:?}", plan.slots[0]);
        assert!(plan.slots[0].room_s > 3.0);
    }

    #[test]
    fn a_slightly_long_line_is_sped_up_within_the_allowed_range() {
        // 2.2s of speech into 2.0s of room: 1.1× — comfortable.
        let plan = plan(&[line(1, 0.0, 2.0, 2.2)], 2.0);
        assert_eq!(plan.slots[0].fit, Fit::Adjusted);
        assert!((plan.slots[0].speed - 1.1).abs() < 0.01);
        assert!((plan.slots[0].length_s - 2.0).abs() < 0.01);
    }

    /// Squeezing a line to double speed to make it fit is worse than letting it run long: the
    /// result is unintelligible, and a viewer would rather read a subtitle a beat late.
    #[test]
    fn a_line_that_would_need_chipmunk_speed_is_allowed_to_run_long_instead() {
        let plan = plan(&[line(1, 0.0, 2.0, 6.0)], 2.0);
        assert_eq!(plan.slots[0].fit, Fit::Overflow);
        assert_eq!(plan.slots[0].speed, MAX_SPEED);
        assert!(plan.slots[0].over_s > 2.0);
        assert!(!plan.is_clean());
    }

    /// Even when it cannot fit, it is played as fast as is still comfortable — overflowing by a
    /// little is better than overflowing by a lot.
    #[test]
    fn an_overflowing_line_is_still_played_at_the_fastest_comfortable_speed() {
        let plan = plan(&[line(1, 0.0, 2.0, 6.0)], 2.0);
        assert!(plan.slots[0].length_s < 6.0);
    }

    #[test]
    fn the_worst_overrun_is_reported_not_the_last_one() {
        let lines = [
            // Back to back, so there is no gap to absorb the first line.
            line(1, 0.0, 1.0, 5.0),
            line(2, 1.0, 2.0, 1.5),
        ];
        let plan = plan(&lines, 3.0);
        assert_eq!(plan.overflows, 1, "the second fits in the gap: {plan:?}");
        assert!(plan.worst_over_s > 2.0);
    }

    /// Without a guard the stretched line butts straight into the next speaker and the two run
    /// together as one breath.
    #[test]
    fn room_stops_short_of_the_next_speaker() {
        let lines = [line(1, 0.0, 1.0, 0.5), line(2, 3.0, 4.0, 0.5)];
        let plan = plan(&lines, 4.0);
        assert!((plan.slots[0].room_s - (3.0 - GUARD_S)).abs() < 1e-9);
    }

    /// A stale duration must not shorten the last line's slot to nothing.
    #[test]
    fn a_total_shorter_than_the_transcript_does_not_crush_the_last_line() {
        let plan = plan(&[line(1, 0.0, 10.0, 9.0)], 2.0);
        assert!(plan.slots[0].room_s >= 10.0);
        assert_eq!(plan.slots[0].fit, Fit::Natural);
    }

    #[test]
    fn a_line_the_synthesiser_produced_nothing_for_does_not_divide_by_zero() {
        let plan = plan(&[line(1, 0.0, 2.0, 0.0)], 2.0);
        assert_eq!(plan.slots[0].speed, 1.0);
        assert_eq!(plan.slots[0].length_s, 0.0);
    }

    #[test]
    fn a_slot_with_no_room_at_all_does_not_produce_infinite_speed() {
        let plan = plan(&[line(1, 5.0, 5.0, 2.0), line(2, 5.0, 6.0, 1.0)], 6.0);
        assert!(plan.slots[0].speed.is_finite());
        assert!(plan.slots[0].speed <= MAX_SPEED);
    }

    #[test]
    fn an_empty_transcript_plans_nothing_rather_than_panicking() {
        let plan = plan(&[], 0.0);
        assert!(plan.slots.is_empty());
        assert!(plan.is_clean());
    }

    #[test]
    fn stretching_to_fill_never_goes_slower_than_is_listenable() {
        // Half a second of speech in ten seconds of room would be 0.05×; clamped.
        assert_eq!(stretch_to_fill(0.5, 10.0), MIN_SPEED);
    }

    #[test]
    fn stretching_never_speeds_a_line_up() {
        assert_eq!(stretch_to_fill(5.0, 2.0), 1.0);
    }
}
