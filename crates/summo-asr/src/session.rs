//! Aggressive pseudo-streaming.
//!
//! A batch model has no partial output: hand it audio, get a transcript of that audio. To make one
//! feel live, [`PseudoSession`] re-decodes the *whole open utterance* every
//! [`SessionConfig::partial_step_ms`] and emits the result as partial text. A five-second sentence
//! is therefore decoded around thirty times instead of once.
//!
//! That sounds wasteful, and it is — deliberately. A model at real-time factor 0.02 uses 2 % of the
//! time budget for a single decode; spending 30× that is still under 60 %, and it converts a model
//! that could only speak after you stopped talking into one that types along with you. The guard
//! rail is the RTF budget, not the multiplier: [`SessionConfig::for_rtf`] derives the cadence from
//! the model's measured speed so a heavier model simply refreshes less often instead of falling
//! behind.
//!
//! Two properties matter more than they look:
//!
//! * **Decoding is stateless per utterance.** The decoder always sees one segment's audio, never a
//!   growing history, so an eight-hour meeting costs exactly what a five-minute one does and there
//!   is no context to drift or overflow.
//! * **Partial text is never trusted.** Only the final decode is filtered for hallucinations and
//!   written to the transcript; partials are cosmetic.

use summo_core::{
    Event, Result,
    audio::{ms_to_samples, samples_to_secs},
    segment::{Lane, Segment, SegmentSource},
};
use summo_vad::gate::{GateConfig, SpeechEvent, VadGate};

use crate::{
    decoder::Decoder,
    hallucination::{HallucinationFilter, Verdict},
};

/// How a session drives its decoder.
#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    pub gate: GateConfig,
    /// Minimum audio added between partial re-decodes. Smaller means more responsive text and more
    /// CPU; the useful range is roughly 100–400 ms.
    pub partial_step_ms: u32,
    pub lane: Lane,
    /// Emit partials at all. Turned off for a refine lane, whose only job is to replace finals.
    pub emit_partials: bool,
    /// Retain each finished utterance's audio so a second, slower model can re-decode it.
    ///
    /// Off by default: holding the audio costs a copy per utterance, and only a hybrid setup needs
    /// it.
    pub keep_final_pcm: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            gate: GateConfig::default(),
            partial_step_ms: 150,
            lane: Lane::Mic,
            emit_partials: true,
            keep_final_pcm: false,
        }
    }
}

impl SessionConfig {
    /// Pick a partial cadence that keeps a model of the given real-time factor inside a CPU budget.
    ///
    /// Re-decoding the open utterance every `step` seconds costs roughly `rtf × utterance_length ÷
    /// step` of a core while someone is speaking. Solving for a target budget gives the cadence, so
    /// a fast model refreshes often and a slow one degrades to fewer refreshes rather than to a
    /// growing backlog.
    #[must_use]
    pub fn for_rtf(rtf: f32, budget: f32) -> Self {
        let budget = budget.clamp(0.05, 0.9);
        // Assume a typical utterance of a few seconds when sizing the step.
        const TYPICAL_UTTERANCE_S: f32 = 3.0;
        let step_s = (rtf * TYPICAL_UTTERANCE_S / budget).clamp(0.1, 2.0);
        Self {
            partial_step_ms: (step_s * 1000.0) as u32,
            ..Self::default()
        }
    }
}

/// Drives one decoder over one audio lane.
pub struct PseudoSession<D: Decoder> {
    decoder: D,
    gate: VadGate,
    cfg: SessionConfig,
    filter: HallucinationFilter,
    /// Samples in the open utterance at the last partial decode.
    last_partial_len: usize,
    /// Decode calls made, for the performance HUD.
    decodes: u64,
    /// Utterances suppressed by the hallucination filter, for diagnostics.
    suppressed: u64,
    /// Audio of the most recent finished utterance, when `keep_final_pcm` is set.
    last_final_pcm: Option<Vec<f32>>,
    /// What the decoder said it heard in that utterance, when it says.
    ///
    /// Beside the audio rather than on the segment: a language is a property of the decode, not of
    /// the transcript, and putting it on `Segment` would send it over the wire and into the vault
    /// for the sake of one routing decision inside this process.
    last_final_language: Option<String>,
}

impl<D: Decoder> PseudoSession<D> {
    #[must_use]
    pub fn new(decoder: D, cfg: SessionConfig) -> Self {
        Self {
            gate: VadGate::new(cfg.gate),
            decoder,
            cfg,
            filter: HallucinationFilter::default(),
            last_partial_len: 0,
            decodes: 0,
            suppressed: 0,
            last_final_pcm: None,
            last_final_language: None,
        }
    }

    /// Take the audio of the most recent finished utterance, if it was retained.
    ///
    /// A hybrid setup uses this to hand the same audio to a slower model without re-buffering it.
    pub fn take_final_pcm(&mut self) -> Option<Vec<f32>> {
        self.last_final_pcm.take()
    }

    /// Take the language the decoder reported for that utterance, if it reported one.
    ///
    /// Taken rather than read, and taken in the same breath as the audio, so a decoder that
    /// answers for one utterance and not the next cannot leave the previous answer behind to be
    /// read as this one's.
    pub fn take_final_language(&mut self) -> Option<String> {
        self.last_final_language.take()
    }

    /// Where the meeting has got to, so a rebuilt pipeline can carry on rather than start over.
    #[must_use]
    pub fn position(&self) -> (u64, usize) {
        self.gate.position()
    }

    /// Take over a meeting already in progress. See [`summo_vad::VadGate::resume_at`].
    pub fn resume_at(&mut self, seq: u64, samples_seen: usize) {
        self.gate.resume_at(seq, samples_seen);
    }

    #[must_use]
    pub fn decode_count(&self) -> u64 {
        self.decodes
    }

    #[must_use]
    pub fn suppressed_count(&self) -> u64 {
        self.suppressed
    }

    #[must_use]
    pub fn decoder_name(&self) -> &str {
        self.decoder.name()
    }

    /// Feed one frame of audio and its speech probability.
    ///
    /// Returns whatever the frame produced: nothing, a partial, or a final.
    pub fn accept(&mut self, frame: &[f32], speech_prob: f32) -> Result<Vec<Event>> {
        let Some(event) = self.gate.feed(frame, speech_prob) else {
            return Ok(Vec::new());
        };

        match event {
            SpeechEvent::Start { .. } => {
                self.decoder.reset();
                self.last_partial_len = 0;
                Ok(Vec::new())
            }
            SpeechEvent::Continue { seq, t0, t1 } => self.maybe_partial(seq, t0, t1),
            SpeechEvent::End {
                seq, t0, t1, pcm, ..
            } => self.finalize(seq, t0, t1, &pcm),
        }
    }

    /// Close any open utterance at the end of a session.
    pub fn flush(&mut self) -> Result<Vec<Event>> {
        let Some(SpeechEvent::End {
            seq, t0, t1, pcm, ..
        }) = self.gate.flush()
        else {
            return Ok(Vec::new());
        };
        self.finalize(seq, t0, t1, &pcm)
    }

    /// Re-decode the open utterance if enough new audio has arrived.
    fn maybe_partial(&mut self, seq: u64, t0: f64, t1: f64) -> Result<Vec<Event>> {
        if !self.cfg.emit_partials || !self.decoder.supports_partials() {
            return Ok(Vec::new());
        }

        let open = self.gate.open_pcm();
        let step = ms_to_samples(self.cfg.partial_step_ms);
        if open.len() < self.last_partial_len + step {
            return Ok(Vec::new());
        }
        self.last_partial_len = open.len();

        // The borrow checker cannot see that `decode` does not touch the gate, so copy the window.
        // At a few seconds of 16 kHz mono this is tens of kilobytes — noise next to the decode.
        let window = open.to_vec();
        self.decodes += 1;
        let transcript = self.decoder.decode(&window)?;

        if transcript.is_empty() {
            return Ok(Vec::new());
        }
        let mut segment = Segment::new(seq, self.cfg.lane, transcript.text, t0, t1);
        segment.source = SegmentSource::Partial;
        segment.conf = transcript.confidence;
        Ok(vec![Event::Partial(segment)])
    }

    /// Decode a closed utterance and emit it, unless it looks invented.
    fn finalize(&mut self, seq: u64, t0: f64, t1: f64, pcm: &[f32]) -> Result<Vec<Event>> {
        self.last_partial_len = 0;
        if self.cfg.keep_final_pcm {
            self.last_final_pcm = Some(pcm.to_vec());
        }
        self.decodes += 1;
        let transcript = self.decoder.decode(pcm)?;
        self.decoder.reset();
        if self.cfg.keep_final_pcm {
            self.last_final_language = transcript.language.clone();
        }

        let verdict = self.filter.judge(&transcript);
        if !verdict.is_keep() {
            self.suppressed += 1;
            tracing::debug!(
                seq,
                lane = self.cfg.lane.as_str(),
                dur_s = samples_to_secs(pcm.len()),
                ?verdict,
                text = %transcript.text,
                "suppressed likely hallucination"
            );
            return Ok(Vec::new());
        }

        let mut segment = Segment::new(seq, self.cfg.lane, transcript.text, t0, t1);
        segment.source = SegmentSource::Final;
        segment.conf = transcript.confidence;
        segment.words = transcript.words;
        Ok(vec![Event::Final(segment)])
    }

    /// Whether an utterance is currently open, for the recording indicator.
    #[must_use]
    pub fn is_speaking(&self) -> bool {
        self.gate.is_speaking()
    }

    /// The hallucination policy, so a hybrid session applies the same rules to refined text.
    #[must_use]
    pub fn filter(&self) -> &HallucinationFilter {
        &self.filter
    }
}

/// Convenience: was this event a final?
#[must_use]
pub fn is_final(event: &Event) -> bool {
    matches!(event, Event::Final(_))
}

/// Convenience: the verdict name, for logs.
#[must_use]
pub fn verdict_name(v: &Verdict) -> &'static str {
    match v {
        Verdict::Keep => "keep",
        Verdict::Boilerplate => "boilerplate",
        Verdict::Repetition => "repetition",
        Verdict::NoSpeech => "no_speech",
        Verdict::Empty => "empty",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{
        Transcript,
        test_support::{FixedDecoder, GrowingDecoder},
    };
    use summo_core::SAMPLE_RATE;

    const FRAME: usize = 160; // 10 ms

    /// Drive a session with a scripted speech/silence pattern.
    fn run<D: Decoder>(session: &mut PseudoSession<D>, script: &[(bool, usize)]) -> Vec<Event> {
        let speech = vec![0.5_f32; FRAME];
        let quiet = vec![0.0_f32; FRAME];
        let mut events = Vec::new();
        for &(is_speech, count) in script {
            for _ in 0..count {
                let (frame, prob) = if is_speech {
                    (&speech, 0.9)
                } else {
                    (&quiet, 0.01)
                };
                events.extend(session.accept(frame, prob).unwrap());
            }
        }
        events
    }

    fn partials(events: &[Event]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Partial(s) => Some(s.text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn finals(events: &[Event]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Final(s) => Some(s.text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_batch_model_produces_growing_partials_then_a_final() {
        let mut s = PseudoSession::new(
            GrowingDecoder::new("một hai ba bốn năm"),
            SessionConfig::default(),
        );
        // 1 s of speech, then silence long enough to close.
        let events = run(&mut s, &[(true, 100), (false, 60)]);

        let partials = partials(&events);
        assert!(
            partials.len() >= 3,
            "expected several partials, got {partials:?}"
        );
        // Text grows monotonically, which is what makes it look like live typing.
        for pair in partials.windows(2) {
            assert!(
                pair[1].len() >= pair[0].len(),
                "partial shrank: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(finals(&events).len(), 1);
    }

    #[test]
    fn cadence_bounds_how_often_the_model_runs() {
        // 2 s of speech at a 500 ms cadence: about four partial decodes plus one final.
        let cfg = SessionConfig {
            partial_step_ms: 500,
            ..SessionConfig::default()
        };
        let mut s = PseudoSession::new(GrowingDecoder::new("một hai ba"), cfg);
        run(&mut s, &[(true, 200), (false, 60)]);

        assert!(
            (4..=7).contains(&s.decode_count()),
            "expected ~5 decodes at 500 ms cadence, got {}",
            s.decode_count()
        );
    }

    #[test]
    fn a_faster_cadence_costs_more_decodes() {
        let mut slow = PseudoSession::new(
            GrowingDecoder::new("một hai ba"),
            SessionConfig {
                partial_step_ms: 500,
                ..SessionConfig::default()
            },
        );
        let mut fast = PseudoSession::new(
            GrowingDecoder::new("một hai ba"),
            SessionConfig {
                partial_step_ms: 100,
                ..SessionConfig::default()
            },
        );
        run(&mut slow, &[(true, 200), (false, 60)]);
        run(&mut fast, &[(true, 200), (false, 60)]);

        assert!(
            fast.decode_count() > slow.decode_count() * 2,
            "cadence should dominate cost: fast={} slow={}",
            fast.decode_count(),
            slow.decode_count()
        );
    }

    #[test]
    fn decoders_that_opt_out_of_partials_only_emit_finals() {
        let mut decoder = GrowingDecoder::new("một hai ba");
        decoder.supports_partials = false;
        let mut s = PseudoSession::new(decoder, SessionConfig::default());

        let events = run(&mut s, &[(true, 100), (false, 60)]);

        assert!(partials(&events).is_empty());
        assert_eq!(finals(&events).len(), 1);
        assert_eq!(s.decode_count(), 1, "no partial decodes should have run");
    }

    #[test]
    fn hallucinated_finals_are_suppressed() {
        // A decoder that returns subtitle boilerplate over what the VAD thought was speech.
        struct Boilerplate;
        impl Decoder for Boilerplate {
            fn decode(&mut self, _pcm: &[f32]) -> Result<Transcript> {
                Ok(Transcript {
                    text: "Thank you.".into(),
                    no_speech_prob: Some(0.8),
                    ..Transcript::default()
                })
            }
            fn name(&self) -> &str {
                "boilerplate"
            }
            fn supports_partials(&self) -> bool {
                false
            }
        }

        let mut s = PseudoSession::new(Boilerplate, SessionConfig::default());
        let events = run(&mut s, &[(true, 100), (false, 60)]);

        assert!(
            finals(&events).is_empty(),
            "boilerplate should not reach the transcript"
        );
        assert_eq!(s.suppressed_count(), 1);
    }

    #[test]
    fn cost_does_not_grow_with_meeting_length() {
        // The claim that makes an eight-hour meeting viable: each utterance is decoded on its own,
        // so the tenth costs what the first did.
        let mut s = PseudoSession::new(FixedDecoder::new("xong"), SessionConfig::default());
        // Lead-in silence so the pre-roll buffer is full before the first utterance, as it is
        // before every later one. Without it the first utterance is genuinely shorter, not cheaper.
        run(&mut s, &[(false, 60)]);

        let mut per_utterance = Vec::new();
        let mut before = 0;
        for _ in 0..10 {
            run(&mut s, &[(true, 50), (false, 60)]);
            per_utterance.push(s.decode_count() - before);
            before = s.decode_count();
        }

        assert_eq!(
            per_utterance.first(),
            per_utterance.last(),
            "decode cost per utterance drifted: {per_utterance:?}"
        );
    }

    #[test]
    fn flush_emits_a_final_for_an_utterance_still_open() {
        let mut s = PseudoSession::new(FixedDecoder::new("chưa xong"), SessionConfig::default());
        let events = run(&mut s, &[(true, 100)]);
        assert!(finals(&events).is_empty(), "still speaking");

        let flushed = s.flush().unwrap();
        assert_eq!(finals(&flushed), vec!["chưa xong"]);
    }

    #[test]
    fn lane_is_carried_onto_every_segment() {
        let cfg = SessionConfig {
            lane: Lane::System,
            ..SessionConfig::default()
        };
        let mut s = PseudoSession::new(FixedDecoder::new("người khác nói"), cfg);
        let events = run(&mut s, &[(true, 100), (false, 60)]);

        let Event::Final(seg) = events.iter().find(|e| is_final(e)).unwrap() else {
            unreachable!()
        };
        assert_eq!(seg.lane, Lane::System);
        assert_eq!(
            seg.speaker, None,
            "remote lane has no speaker until diarization runs"
        );
    }

    #[test]
    fn sequence_numbers_are_stable_from_partial_to_final() {
        let mut s = PseudoSession::new(GrowingDecoder::new("một hai ba"), SessionConfig::default());
        let events = run(&mut s, &[(true, 100), (false, 60)]);

        let seqs: Vec<u64> = events
            .iter()
            .filter_map(|e| e.segment().map(|s| s.seq))
            .collect();
        assert!(!seqs.is_empty());
        assert!(
            seqs.iter().all(|&s| s == seqs[0]),
            "one utterance must keep one seq: {seqs:?}"
        );
    }

    #[test]
    fn cadence_from_rtf_slows_down_for_heavier_models() {
        let light = SessionConfig::for_rtf(0.02, 0.5);
        let heavy = SessionConfig::for_rtf(0.30, 0.5);
        assert!(
            heavy.partial_step_ms > light.partial_step_ms,
            "a slower model must refresh less often: {} vs {}",
            heavy.partial_step_ms,
            light.partial_step_ms
        );
        assert!((100..=2000).contains(&light.partial_step_ms));
        assert!((100..=2000).contains(&heavy.partial_step_ms));
    }

    #[test]
    fn silence_alone_never_calls_the_decoder() {
        let mut s = PseudoSession::new(
            FixedDecoder::new("không nên xuất hiện"),
            SessionConfig::default(),
        );
        let events = run(&mut s, &[(false, 500)]);
        assert!(events.is_empty());
        assert_eq!(s.decode_count(), 0, "an idle meeting must cost nothing");
    }

    #[test]
    fn partial_window_length_tracks_the_open_utterance() {
        // Guards the pseudo-streaming premise: the decoder sees the whole utterance so far, not the
        // newest slice, because a batch model cannot stitch increments.
        struct LengthSpy {
            seen: Vec<usize>,
        }
        impl Decoder for LengthSpy {
            fn decode(&mut self, pcm: &[f32]) -> Result<Transcript> {
                self.seen.push(pcm.len());
                Ok(Transcript::new("x"))
            }
            fn name(&self) -> &str {
                "spy"
            }
        }

        let mut s = PseudoSession::new(LengthSpy { seen: Vec::new() }, SessionConfig::default());
        // Lead-in silence fills the pre-roll ring, so the first window carries the utterance's
        // onset rather than starting at the frame the VAD happened to react on.
        run(&mut s, &[(false, 60), (true, 100), (false, 60)]);

        let seen = &s.decoder.seen;
        assert!(seen.len() >= 3);
        for pair in seen[..seen.len() - 1].windows(2) {
            assert!(pair[1] > pair[0], "window should grow: {seen:?}");
        }
        assert!(
            seen[0] >= SAMPLE_RATE as usize / 4,
            "first window should already include pre-roll, got {} samples",
            seen[0]
        );
    }
}
