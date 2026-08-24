//! Utterance segmentation.
//!
//! The gate converts a stream of per-frame speech probabilities into utterance boundaries. Three
//! details carry almost all of its value, and all three were learned the expensive way in the
//! Python prototype:
//!
//! * **Pre-roll.** A VAD reports speech a beat after it actually starts, so the first consonant is
//!   already gone by the time the gate reacts. The gate therefore keeps the last
//!   [`GateConfig::preroll_ms`] of audio at all times and prepends it to every segment.
//! * **Hysteresis via minimum durations.** Fan noise trips a single frame constantly. Requiring
//!   [`GateConfig::min_speech_s`] of speech before opening, and
//!   [`GateConfig::min_silence_s`] of quiet before closing, is what stops the transcript
//!   fragmenting into noise.
//! * **When the segment closes is when the user sees final text.** Trailing-silence latency is
//!   added directly to perceived finalize latency, which is the whole reason a faster-releasing VAD
//!   backend is worth benchmarking.

use serde::{Deserialize, Serialize};
use summo_core::audio::{SAMPLE_RATE, samples_to_secs, secs_to_samples};

/// Tuning for [`VadGate`].
///
/// Defaults are the production preset measured on laptop microphones with fan and air-conditioner
/// noise present.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GateConfig {
    /// Speech probability above which a frame counts as speech.
    pub threshold: f32,
    /// Speech required before an utterance opens. Rejects single-frame noise spikes.
    pub min_speech_s: f32,
    /// Trailing silence required before an utterance closes. Too short truncates sentences; too
    /// long makes finals feel sluggish.
    pub min_silence_s: f32,
    /// Audio retained before the first speech frame.
    pub preroll_ms: u32,
    /// Hard cap on one utterance, so a monologue still produces finals instead of one giant
    /// segment at the end of the meeting.
    pub max_segment_s: f32,
    /// Silence kept at the end of a closed segment. A little tail helps decoders that use right
    /// context; the rest is dropped so the segment's `t1` reflects when speech stopped.
    pub tail_ms: u32,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            threshold: 0.35,
            min_speech_s: 0.12,
            min_silence_s: 0.40,
            preroll_ms: 400,
            max_segment_s: 30.0,
            tail_ms: 200,
        }
    }
}

impl GateConfig {
    /// Preset for noisy rooms: harder to trip, slower to close.
    #[must_use]
    pub fn noisy() -> Self {
        Self {
            threshold: 0.55,
            min_speech_s: 0.20,
            min_silence_s: 0.60,
            ..Self::default()
        }
    }

    /// Preset that favours latency over sentence integrity.
    #[must_use]
    pub fn fast() -> Self {
        Self {
            min_silence_s: 0.25,
            preroll_ms: 300,
            ..Self::default()
        }
    }

    fn validate(&self) -> Self {
        Self {
            threshold: self.threshold.clamp(0.05, 0.95),
            min_speech_s: self.min_speech_s.max(0.0),
            min_silence_s: self.min_silence_s.max(0.05),
            preroll_ms: self.preroll_ms.min(5_000),
            max_segment_s: if self.max_segment_s <= 0.0 {
                30.0
            } else {
                self.max_segment_s
            },
            tail_ms: self.tail_ms.min(2_000),
        }
    }
}

/// What the gate decided after consuming a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum SpeechEvent {
    /// An utterance opened. `t0` includes the pre-roll, so it precedes the triggering frame.
    Start { seq: u64, t0: f64 },
    /// The open utterance grew. Emitted on every frame while speech is open; the caller decides how
    /// often to actually re-decode.
    Continue { seq: u64, t0: f64, t1: f64 },
    /// The utterance closed. `pcm` is the complete audio including pre-roll and tail.
    End {
        seq: u64,
        t0: f64,
        t1: f64,
        pcm: Vec<f32>,
        reason: EndReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// Trailing silence exceeded `min_silence_s` — the normal path.
    Silence,
    /// Hit `max_segment_s` mid-speech; a new segment opens immediately after.
    MaxLength,
    /// Session ended with an utterance still open.
    Flush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No utterance open. Frames go to the pre-roll ring only.
    Idle,
    /// Speech detected but not yet long enough to open an utterance.
    Arming,
    /// Utterance open.
    Speaking,
}

/// Segmentation state machine. Backend-agnostic: it consumes probabilities, not audio models.
#[derive(Debug)]
pub struct VadGate {
    cfg: GateConfig,
    state: State,
    /// Audio for the currently open (or arming) utterance.
    buf: Vec<f32>,
    /// Rolling pre-roll window kept while idle.
    preroll: std::collections::VecDeque<f32>,
    preroll_cap: usize,
    /// Total samples consumed since session start; the clock for all timestamps.
    samples_seen: usize,
    /// Sample index where the current buffer starts.
    buf_start: usize,
    /// Consecutive speech samples while arming.
    arming_speech: usize,
    /// Consecutive silence samples while speaking.
    trailing_silence: usize,
    /// Samples of speech seen in the open utterance, used to reject all-noise segments.
    speech_in_segment: usize,
    seq: u64,
}

impl VadGate {
    #[must_use]
    pub fn new(cfg: GateConfig) -> Self {
        let cfg = cfg.validate();
        let preroll_cap = (SAMPLE_RATE as usize * cfg.preroll_ms as usize) / 1000;
        Self {
            cfg,
            state: State::Idle,
            buf: Vec::with_capacity(SAMPLE_RATE as usize * 4),
            preroll: std::collections::VecDeque::with_capacity(preroll_cap + 1),
            preroll_cap,
            samples_seen: 0,
            buf_start: 0,
            arming_speech: 0,
            trailing_silence: 0,
            speech_in_segment: 0,
            seq: 0,
        }
    }

    #[must_use]
    pub fn config(&self) -> GateConfig {
        self.cfg
    }

    /// Where this gate has got to: the next sequence number, and the clock behind it.
    ///
    /// Both belong to the *meeting*, not to the pipeline decoding it — and a mid-meeting model or
    /// language change rebuilds the pipeline. Without carrying these across, the new gate started
    /// at zero: fresh utterances were numbered from 0 again, collided with lines already on screen,
    /// and the transcript — which indexes by sequence number — overwrote its own beginning. Every
    /// timestamp restarted with them, so a recording that had reached 00:24 went back to 00:00.
    /// Reported as "the time keeps looping, and changing the language puts it back at the top".
    #[must_use]
    pub fn position(&self) -> (u64, usize) {
        (self.seq, self.samples_seen)
    }

    /// Continue a meeting that a new pipeline is taking over.
    ///
    /// Only ever called on a gate that has consumed nothing. Seeding one mid-flight would move the
    /// clock under an utterance it is already buffering.
    pub fn resume_at(&mut self, seq: u64, samples_seen: usize) {
        debug_assert!(
            self.samples_seen == 0 && matches!(self.state, State::Idle),
            "a gate is only resumed before it has heard anything"
        );
        self.seq = seq;
        self.samples_seen = samples_seen;
        self.buf_start = samples_seen;
    }

    /// Audio accumulated for the open utterance, for partial re-decodes.
    #[must_use]
    pub fn open_pcm(&self) -> &[f32] {
        if self.state == State::Speaking {
            &self.buf
        } else {
            &[]
        }
    }

    /// Whether an utterance is currently open.
    #[must_use]
    pub fn is_speaking(&self) -> bool {
        self.state == State::Speaking
    }

    /// Session-relative time of the most recent sample consumed.
    #[must_use]
    pub fn now(&self) -> f64 {
        samples_to_secs(self.samples_seen)
    }

    /// Consume one frame and its speech probability.
    ///
    /// `frame` must be the audio the probability was computed from; the gate stores it verbatim.
    /// Returns at most one event — the caller drives partial decodes off [`SpeechEvent::Continue`].
    pub fn feed(&mut self, frame: &[f32], prob: f32) -> Option<SpeechEvent> {
        let is_speech = prob >= self.cfg.threshold;
        let n = frame.len();
        self.samples_seen += n;

        match self.state {
            State::Idle => {
                self.push_preroll(frame);
                if is_speech {
                    // Open a buffer seeded with pre-roll so the utterance keeps its onset.
                    self.buf.clear();
                    self.buf.extend(self.preroll.iter().copied());
                    self.buf_start = self.samples_seen.saturating_sub(self.buf.len());
                    self.arming_speech = n;
                    self.speech_in_segment = n;
                    self.trailing_silence = 0;
                    self.state = State::Arming;
                    // The frame is already in `buf` via pre-roll; do not append it twice.
                    self.maybe_open()
                } else {
                    None
                }
            }
            State::Arming => {
                self.buf.extend_from_slice(frame);
                self.push_preroll(frame);
                if is_speech {
                    self.arming_speech += n;
                    self.speech_in_segment += n;
                    self.maybe_open()
                } else {
                    // A lone spike: abandon the candidate and go back to idle.
                    self.state = State::Idle;
                    self.buf.clear();
                    self.arming_speech = 0;
                    self.speech_in_segment = 0;
                    None
                }
            }
            State::Speaking => {
                self.buf.extend_from_slice(frame);
                self.push_preroll(frame);
                if is_speech {
                    self.trailing_silence = 0;
                    self.speech_in_segment += n;
                } else {
                    self.trailing_silence += n;
                }

                if self.trailing_silence >= secs_to_samples(f64::from(self.cfg.min_silence_s)) {
                    return self.close(EndReason::Silence);
                }
                if self.buf.len() >= secs_to_samples(f64::from(self.cfg.max_segment_s)) {
                    return self.close(EndReason::MaxLength);
                }
                Some(SpeechEvent::Continue {
                    seq: self.seq,
                    t0: samples_to_secs(self.buf_start),
                    t1: self.now(),
                })
            }
        }
    }

    /// Close any open utterance at end of session.
    pub fn flush(&mut self) -> Option<SpeechEvent> {
        match self.state {
            State::Speaking => self.close(EndReason::Flush),
            // An arming candidate never reached `min_speech_s`; dropping it is the point.
            State::Arming | State::Idle => {
                self.state = State::Idle;
                self.buf.clear();
                None
            }
        }
    }

    /// Clear all state, including the session clock.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.buf.clear();
        self.preroll.clear();
        self.samples_seen = 0;
        self.buf_start = 0;
        self.arming_speech = 0;
        self.trailing_silence = 0;
        self.speech_in_segment = 0;
        self.seq = 0;
    }

    fn maybe_open(&mut self) -> Option<SpeechEvent> {
        if self.arming_speech < secs_to_samples(f64::from(self.cfg.min_speech_s)) {
            return None;
        }
        self.state = State::Speaking;
        self.trailing_silence = 0;
        Some(SpeechEvent::Start {
            seq: self.seq,
            t0: samples_to_secs(self.buf_start),
        })
    }

    fn close(&mut self, reason: EndReason) -> Option<SpeechEvent> {
        let keep_tail = (SAMPLE_RATE as usize * self.cfg.tail_ms as usize) / 1000;
        // Drop trailing silence beyond the configured tail so `t1` marks when speech stopped.
        let drop = self
            .trailing_silence
            .saturating_sub(keep_tail)
            .min(self.buf.len());
        let keep = self.buf.len() - drop;

        let mut pcm = std::mem::take(&mut self.buf);
        pcm.truncate(keep);

        let seq = self.seq;
        let t0 = samples_to_secs(self.buf_start);
        let t1 = samples_to_secs(self.buf_start + keep);

        self.seq += 1;
        self.trailing_silence = 0;
        self.arming_speech = 0;
        let had_speech = self.speech_in_segment;
        self.speech_in_segment = 0;
        self.state = State::Idle;
        self.buf = Vec::with_capacity(SAMPLE_RATE as usize * 4);

        // A segment that never accumulated `min_speech_s` of speech is noise; suppress it rather
        // than handing the decoder audio that will come back as a hallucinated phrase.
        if had_speech < secs_to_samples(f64::from(self.cfg.min_speech_s)) {
            return None;
        }

        Some(SpeechEvent::End {
            seq,
            t0,
            t1,
            pcm,
            reason,
        })
    }

    fn push_preroll(&mut self, frame: &[f32]) {
        if self.preroll_cap == 0 {
            return;
        }
        for &s in frame {
            if self.preroll.len() == self.preroll_cap {
                self.preroll.pop_front();
            }
            self.preroll.push_back(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: usize = 160; // 10 ms, TEN-VAD's smallest hop

    /// Drive the gate with a scripted pattern of speech/silence frames.
    ///
    /// `script` is a list of `(is_speech, frame_count)` runs. Audio is a constant so segment
    /// contents can be checked by length alone.
    fn run(cfg: GateConfig, script: &[(bool, usize)]) -> (Vec<SpeechEvent>, VadGate) {
        let mut gate = VadGate::new(cfg);
        let speech_frame = vec![0.5_f32; FRAME];
        let quiet_frame = vec![0.0_f32; FRAME];
        let mut events = Vec::new();
        for &(is_speech, count) in script {
            for _ in 0..count {
                let (frame, prob) = if is_speech {
                    (&speech_frame, 0.9)
                } else {
                    (&quiet_frame, 0.01)
                };
                if let Some(ev) = gate.feed(frame, prob) {
                    events.push(ev);
                }
            }
        }
        (events, gate)
    }

    fn ends(events: &[SpeechEvent]) -> Vec<&SpeechEvent> {
        events
            .iter()
            .filter(|e| matches!(e, SpeechEvent::End { .. }))
            .collect()
    }

    fn starts(events: &[SpeechEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, SpeechEvent::Start { .. }))
            .count()
    }

    #[test]
    fn single_noise_spike_never_opens_a_segment() {
        // 10 ms of "speech" against a 120 ms minimum: this is the fan tripping the detector.
        let (events, _) = run(
            GateConfig::default(),
            &[(false, 50), (true, 1), (false, 100)],
        );
        assert_eq!(starts(&events), 0);
        assert!(ends(&events).is_empty());
    }

    #[test]
    fn sustained_speech_opens_and_silence_closes() {
        let (events, _) = run(
            GateConfig::default(),
            &[(false, 30), (true, 100), (false, 60)],
        );
        assert_eq!(starts(&events), 1);
        let ends = ends(&events);
        assert_eq!(ends.len(), 1);
        let SpeechEvent::End { reason, t0, t1, .. } = ends[0] else {
            unreachable!()
        };
        assert_eq!(*reason, EndReason::Silence);
        assert!(t1 > t0);
    }

    #[test]
    fn preroll_is_prepended_so_the_onset_is_not_clipped() {
        let cfg = GateConfig::default();
        let (events, _) = run(cfg, &[(false, 100), (true, 100), (false, 60)]);
        let SpeechEvent::Start { t0, .. } = events[0] else {
            panic!("expected a Start first, got {:?}", events[0])
        };
        // Speech began at 1.0 s; the segment must start ~400 ms earlier.
        let expected = 1.0 - f64::from(cfg.preroll_ms) / 1000.0;
        assert!(
            (t0 - expected).abs() < 0.02,
            "segment started at {t0}, expected about {expected}"
        );
    }

    #[test]
    fn short_gaps_inside_a_sentence_do_not_split_it() {
        // 200 ms pause mid-sentence, under the 400 ms close threshold.
        let (events, _) = run(
            GateConfig::default(),
            &[(true, 60), (false, 20), (true, 60), (false, 60)],
        );
        assert_eq!(starts(&events), 1, "a breath must not split the utterance");
        assert_eq!(ends(&events).len(), 1);
    }

    #[test]
    fn trailing_silence_is_trimmed_to_the_tail() {
        let cfg = GateConfig::default();
        let (events, _) = run(cfg, &[(true, 100), (false, 100)]);
        let SpeechEvent::End { t1, .. } = ends(&events)[0] else {
            unreachable!()
        };
        // Speech stopped at 1.0 s; only `tail_ms` of silence should survive.
        let expected = 1.0 + f64::from(cfg.tail_ms) / 1000.0;
        assert!(
            (t1 - expected).abs() < 0.05,
            "segment ended at {t1}, expected about {expected}"
        );
    }

    #[test]
    fn long_monologue_is_split_at_max_length() {
        let cfg = GateConfig {
            max_segment_s: 2.0,
            ..GateConfig::default()
        };
        // 10 s of unbroken speech.
        let (events, _) = run(cfg, &[(true, 1000), (false, 60)]);
        let ends = ends(&events);
        assert!(
            ends.len() >= 4,
            "expected repeated splits, got {}",
            ends.len()
        );
        assert!(
            ends.iter().any(|e| matches!(
                e,
                SpeechEvent::End {
                    reason: EndReason::MaxLength,
                    ..
                }
            )),
            "at least one split must be attributed to max length"
        );
    }

    #[test]
    fn flush_closes_an_open_utterance() {
        let (mut events, mut gate) = run(GateConfig::default(), &[(true, 100)]);
        assert!(
            ends(&events).is_empty(),
            "still speaking, nothing closed yet"
        );
        events.extend(gate.flush());
        let ends = ends(&events);
        assert_eq!(ends.len(), 1);
        assert!(matches!(
            ends[0],
            SpeechEvent::End {
                reason: EndReason::Flush,
                ..
            }
        ));
    }

    #[test]
    fn flush_discards_a_candidate_that_never_qualified() {
        let (_, mut gate) = run(GateConfig::default(), &[(false, 50), (true, 1)]);
        assert!(gate.flush().is_none());
    }

    #[test]
    fn sequence_numbers_increment_per_segment() {
        let (events, _) = run(
            GateConfig::default(),
            &[(true, 60), (false, 60), (true, 60), (false, 60)],
        );
        let seqs: Vec<u64> = ends(&events)
            .iter()
            .map(|e| match e {
                SpeechEvent::End { seq, .. } => *seq,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(seqs, vec![0, 1]);
    }

    #[test]
    fn open_pcm_grows_while_speaking_and_empties_after_close() {
        let mut gate = VadGate::new(GateConfig::default());
        let frame = vec![0.5_f32; FRAME];
        for _ in 0..60 {
            gate.feed(&frame, 0.9);
        }
        assert!(gate.is_speaking());
        let mid = gate.open_pcm().len();
        assert!(mid > 0);
        for _ in 0..60 {
            gate.feed(&vec![0.0; FRAME], 0.0);
        }
        assert!(!gate.is_speaking());
        assert!(gate.open_pcm().is_empty());
    }

    #[test]
    fn faster_release_lowers_finalize_latency() {
        // The whole reason a snappier VAD backend is worth shipping: closing sooner means the user
        // sees final text sooner, for identical audio.
        let script = [(true, 60), (false, 100)];
        let slow = run(
            GateConfig {
                min_silence_s: 0.7,
                ..GateConfig::default()
            },
            &script,
        );
        let fast = run(
            GateConfig {
                min_silence_s: 0.25,
                ..GateConfig::default()
            },
            &script,
        );

        let close_time = |(events, _): &(Vec<SpeechEvent>, VadGate)| -> f64 {
            events
                .iter()
                .find_map(|e| match e {
                    SpeechEvent::End { t1, .. } => Some(*t1),
                    _ => None,
                })
                .expect("segment should close")
        };
        assert!(close_time(&fast) <= close_time(&slow));
    }

    #[test]
    fn config_is_clamped_to_sane_ranges() {
        let gate = VadGate::new(GateConfig {
            threshold: 9.0,
            min_silence_s: -1.0,
            max_segment_s: 0.0,
            ..GateConfig::default()
        });
        let cfg = gate.config();
        assert!(cfg.threshold <= 0.95);
        assert!(cfg.min_silence_s >= 0.05);
        assert!(cfg.max_segment_s > 0.0);
    }

    #[test]
    fn reset_clears_the_session_clock() {
        let (_, mut gate) = run(GateConfig::default(), &[(true, 60), (false, 60)]);
        assert!(gate.now() > 0.0);
        gate.reset();
        assert_eq!(gate.now(), 0.0);
        assert!(!gate.is_speaking());
    }
}

#[cfg(test)]
mod carrying_a_meeting {
    use super::*;

    /// A rebuilt pipeline continues the meeting instead of starting a new one.
    ///
    /// Changing the model or the spoken language mid-recording builds a fresh `SessionRunner`, and
    /// every gate in it used to begin at sequence zero with a zeroed clock. The transcript indexes
    /// by sequence number, so the next utterance after a swap was numbered 0 — the same as the
    /// first thing anybody said — and overwrote it, at the top, with a timestamp of 00:00 on a
    /// recording that had reached 00:24.
    #[test]
    fn a_resumed_gate_keeps_numbering_and_timing_from_where_the_last_one_stopped() {
        let fresh = VadGate::new(GateConfig::default());
        assert_eq!(
            fresh.position(),
            (0, 0),
            "a new meeting starts at the start"
        );

        let mut taking_over = VadGate::new(GateConfig::default());
        taking_over.resume_at(7, SAMPLE_RATE as usize * 24);
        assert_eq!(
            taking_over.position(),
            (7, SAMPLE_RATE as usize * 24),
            "the next utterance is the eighth, not the first"
        );
    }

    /// And `reset` still means "a new stream", which is a different thing from a swap: the same
    /// gate reused for another meeting has to forget.
    #[test]
    fn reset_still_goes_back_to_the_beginning() {
        let mut gate = VadGate::new(GateConfig::default());
        gate.resume_at(7, SAMPLE_RATE as usize * 24);
        gate.reset();
        assert_eq!(gate.position().0, 0);
    }
}
