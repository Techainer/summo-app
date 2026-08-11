//! Pipeline stages that need a model.
//!
//! [`summo_pipeline`] deliberately carries no ONNX dependency, so the stages that load one live
//! here. They are written against the `Vad` and `Decoder` *traits* rather than against Silero and
//! sherpa specifically, which is what lets the tests below drive the real stage logic with fakes —
//! no model files, no native library, on any machine.
//!
//! ## Why this exists next to `SessionRunner`
//!
//! `SessionRunner` is the production path and still is. It hardcodes framer → detector → decoder →
//! clusterer per lane, which works and cannot be extended: live translation ended up bolted onto
//! the socket handler because there was nowhere in the runner to put it.
//!
//! These stages are that shape as composable pieces. They are not yet what the daemon runs — that
//! migration needs a recording rig this machine does not have, and swapping the transcription path
//! on an argument rather than on a measurement is how a working pipeline stops working quietly.
//! What is proven here is that the stages behave: the tests drive gating, buffering, flushing and
//! lane separation with fakes, which is exactly the logic a migration would be trusting.

use summo_asr::{Decoder, PseudoSession, SessionConfig};
use summo_core::{Result, segment::Lane};
use summo_pipeline::{Frame, Processor};
use summo_vad::Vad;

/// Score every audio frame for speech, and pass the audio on.
///
/// Emits a [`Frame::Voice`] *after* the audio it describes rather than replacing it: the decoder
/// downstream needs both, and a stage that swapped one for the other would force every later stage
/// to reconstruct what it threw away.
///
/// One detector per lane. Silero holds recurrent state, and two speakers sharing it would leave
/// each one's silence deciding the other's gate.
pub struct Detect {
    lane: Lane,
    vad: Box<dyn Vad>,
}

impl Detect {
    #[must_use]
    pub fn new(lane: Lane, vad: Box<dyn Vad>) -> Self {
        Self { lane, vad }
    }

    /// The exact frame width this detector wants, for the [`summo_pipeline::processors::Reframe`]
    /// that has to sit in front of it.
    #[must_use]
    pub fn frame_len(&self) -> usize {
        self.vad.frame_len()
    }
}

impl Processor for Detect {
    fn name(&self) -> &'static str {
        "detect"
    }

    fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        let Frame::Audio(audio) = &frame else {
            return Ok(vec![frame]);
        };
        // Another lane's audio: not this detector's business, and feeding it would corrupt the
        // recurrent state with a different speaker's silence.
        if audio.lane != self.lane {
            return Ok(vec![frame]);
        }

        let probability = self.vad.feed_frame(&audio.samples)?;
        Ok(vec![
            frame,
            Frame::Voice {
                lane: self.lane,
                probability,
            },
        ])
    }

    fn reset(&mut self) {
        self.vad.reset();
    }
}

/// Turn gated audio into transcript events.
///
/// Wraps [`PseudoSession`], which already owns the hard parts: opening an utterance on speech,
/// closing it on trailing silence, re-decoding a growing window for partials, and suppressing the
/// text a model hallucinates over silence. This is the adapter that makes it a pipeline stage.
///
/// It consumes audio and voice frames and emits only events. A stage downstream of this one is
/// working with transcript, not with sound — which is exactly where a translator belongs.
pub struct Recognise {
    lane: Lane,
    session: PseudoSession<Box<dyn Decoder>>,
    /// The audio for the frame whose probability has not arrived yet.
    ///
    /// `Detect` emits audio then probability, so this stage always has one frame in hand. Pairing
    /// them here rather than upstream keeps the two stages independent: a pipeline can run the
    /// detector without the decoder, which is what a "is my microphone working" screen needs.
    pending: Option<Vec<f32>>,
}

impl Recognise {
    #[must_use]
    pub fn new(lane: Lane, decoder: Box<dyn Decoder>, config: SessionConfig) -> Self {
        Self {
            lane,
            session: PseudoSession::new(decoder, config),
            pending: None,
        }
    }

    #[must_use]
    pub fn decode_count(&self) -> u64 {
        self.session.decode_count()
    }

    #[must_use]
    pub fn suppressed_count(&self) -> u64 {
        self.session.suppressed_count()
    }

    #[must_use]
    pub fn is_speaking(&self) -> bool {
        self.session.is_speaking()
    }
}

impl Processor for Recognise {
    fn name(&self) -> &'static str {
        "recognise"
    }

    fn push(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        match frame {
            Frame::Audio(audio) if audio.lane == self.lane => {
                self.pending = Some(audio.samples);
                // Consumed: downstream stages work with transcript, not sound.
                Ok(vec![])
            }
            Frame::Voice { lane, probability } if lane == self.lane => {
                let Some(samples) = self.pending.take() else {
                    // A probability with no audio in front of it means somebody rewired the chain.
                    // Dropping it is better than decoding the previous frame twice.
                    tracing::debug!("voice frame with no audio before it; ignoring");
                    return Ok(vec![]);
                };
                let events = self.session.accept(&samples, probability)?;
                Ok(events.into_iter().map(Frame::Event).collect())
            }
            Frame::Flush | Frame::End => {
                // The open utterance, before the control frame that ended it — otherwise the last
                // sentence of every meeting arrives after the sink has stopped listening.
                let mut out: Vec<Frame> = self
                    .session
                    .flush()?
                    .into_iter()
                    .map(Frame::Event)
                    .collect();
                out.push(frame);
                Ok(out)
            }
            other => Ok(vec![other]),
        }
    }

    fn reset(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::Event;
    use summo_pipeline::{Pipeline, processors::Reframe};

    /// Speech whenever the frame is loud, silence when it is not. Enough to drive the gate.
    struct LoudIsSpeech {
        width: usize,
        resets: usize,
    }

    impl Vad for LoudIsSpeech {
        fn name(&self) -> &'static str {
            "loud-is-speech"
        }
        fn frame_len(&self) -> usize {
            self.width
        }
        fn feed_frame(&mut self, frame: &[f32]) -> Result<f32> {
            let peak = frame.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            Ok(if peak > 0.1 { 0.95 } else { 0.02 })
        }
        fn reset(&mut self) {
            self.resets += 1;
        }
    }

    /// Returns fixed text, so a test can assert on segments without a model.
    struct SaysHello {
        calls: usize,
    }

    impl Decoder for SaysHello {
        fn name(&self) -> &str {
            "says-hello"
        }
        fn decode(&mut self, _samples: &[f32]) -> Result<summo_asr::Transcript> {
            self.calls += 1;
            Ok(summo_asr::Transcript::new("xin chào"))
        }
    }

    fn detector(lane: Lane) -> Detect {
        Detect::new(lane, Box::new(LoudIsSpeech { width: 160, resets: 0 }))
    }

    fn recogniser(lane: Lane) -> Recognise {
        Recognise::new(
            lane,
            Box::new(SaysHello { calls: 0 }),
            SessionConfig {
                lane,
                ..SessionConfig::default()
            },
        )
    }

    fn loud(lane: Lane, n: usize) -> Frame {
        Frame::audio(lane, vec![0.8; n], 16_000)
    }

    fn quiet(lane: Lane, n: usize) -> Frame {
        Frame::audio(lane, vec![0.0; n], 16_000)
    }

    fn events(frames: &[Frame]) -> Vec<&Event> {
        frames
            .iter()
            .filter_map(|f| match f {
                Frame::Event(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    /// The audio has to survive: the decoder downstream needs it, and a stage that replaced it
    /// would force every later stage to reconstruct what it threw away.
    #[test]
    fn detect_scores_a_frame_and_keeps_the_audio() {
        let mut d = detector(Lane::Mic);
        let out = d.push(loud(Lane::Mic, 160)).unwrap();

        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], Frame::Audio(_)), "audio first");
        match out[1] {
            Frame::Voice { probability, .. } => assert!(probability > 0.5),
            _ => panic!("expected a voice frame after the audio"),
        }
    }

    #[test]
    fn detect_reports_silence_as_silence() {
        let mut d = detector(Lane::Mic);
        let out = d.push(quiet(Lane::Mic, 160)).unwrap();
        match out[1] {
            Frame::Voice { probability, .. } => assert!(probability < 0.5),
            _ => panic!("expected a voice frame"),
        }
    }

    /// Two speakers sharing a detector would leave each one's silence deciding the other's gate.
    #[test]
    fn detect_ignores_another_lanes_audio() {
        let mut d = detector(Lane::Mic);
        let out = d.push(loud(Lane::System, 160)).unwrap();
        assert_eq!(out.len(), 1, "forwarded untouched, not scored");
    }

    #[test]
    fn detect_forwards_control_frames() {
        let mut d = detector(Lane::Mic);
        assert_eq!(d.push(Frame::End).unwrap(), vec![Frame::End]);
    }

    /// A stage downstream of recognition works with transcript, not with sound.
    #[test]
    fn recognise_consumes_audio_and_emits_only_events() {
        let mut r = recogniser(Lane::Mic);
        assert!(r.push(loud(Lane::Mic, 160)).unwrap().is_empty());

        let out = r
            .push(Frame::Voice {
                lane: Lane::Mic,
                probability: 0.95,
            })
            .unwrap();
        assert!(out.iter().all(|f| matches!(f, Frame::Event(_))));
    }

    /// The last sentence of every meeting would otherwise arrive after the sink stopped listening.
    #[test]
    fn recognise_flushes_the_open_utterance_before_the_control_frame() {
        let mut r = recogniser(Lane::Mic);
        for _ in 0..20 {
            r.push(loud(Lane::Mic, 160)).unwrap();
            r.push(Frame::Voice {
                lane: Lane::Mic,
                probability: 0.95,
            })
            .unwrap();
        }

        let out = r.push(Frame::End).unwrap();
        assert!(out.last().unwrap().is_end(), "the end goes last");
        assert!(
            events(&out).iter().any(|e| matches!(e, Event::Final(_))),
            "and the open utterance came out first: {out:?}"
        );
    }

    /// Rewiring the chain must not decode the previous frame twice.
    #[test]
    fn recognise_ignores_a_voice_frame_with_no_audio_before_it() {
        let mut r = recogniser(Lane::Mic);
        let out = r
            .push(Frame::Voice {
                lane: Lane::Mic,
                probability: 0.95,
            })
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn recognise_leaves_another_lane_alone() {
        let mut r = recogniser(Lane::Mic);
        assert_eq!(r.push(loud(Lane::System, 160)).unwrap().len(), 1);
    }

    /// The whole point of the exercise: the transcription path as an assembled chain, driven with
    /// fakes on a machine with no models on it.
    #[test]
    fn a_full_chain_turns_audio_into_a_transcript() {
        let width = detector(Lane::Mic).frame_len();
        let mut pipeline = Pipeline::new()
            .then(Reframe::new(width, 16_000))
            .then(detector(Lane::Mic))
            .then(recogniser(Lane::Mic));

        assert_eq!(pipeline.describe(), "reframe → detect → recognise");

        // Speech, then enough silence to close the utterance.
        let mut produced = Vec::new();
        produced.extend(pipeline.push(loud(Lane::Mic, width * 30)).unwrap());
        produced.extend(pipeline.push(quiet(Lane::Mic, width * 40)).unwrap());
        produced.extend(pipeline.push(Frame::End).unwrap());

        let finals: Vec<&Event> = events(&produced)
            .into_iter()
            .filter(|e| matches!(e, Event::Final(_)))
            .collect();

        assert!(!finals.is_empty(), "expected a committed utterance: {produced:?}");
        match finals[0] {
            Event::Final(segment) => {
                assert_eq!(segment.text, "xin chào");
                assert_eq!(segment.lane, Lane::Mic);
            }
            _ => unreachable!(),
        }
        assert!(produced.iter().any(Frame::is_end), "and the stream ended");
    }

    /// One pipeline serving a second recording is the reason `reset` exists rather than rebuilding
    /// the chain — rebuilding means reloading the models.
    #[test]
    fn a_chain_can_be_reset_and_used_again() {
        let width = 160;
        let mut pipeline = Pipeline::new()
            .then(Reframe::new(width, 16_000))
            .then(detector(Lane::Mic))
            .then(recogniser(Lane::Mic));

        pipeline.push(loud(Lane::Mic, width * 10)).unwrap();
        pipeline.reset();

        // Nothing buffered from the first run leaks into the second.
        let out = pipeline.push(Frame::Flush).unwrap();
        assert!(
            !out.iter().any(|f| matches!(f, Frame::Audio(_))),
            "the reframer forgot its remainder: {out:?}"
        );
    }
}
