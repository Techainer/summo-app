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

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use summo_asr::{Decoder, HybridSession, PseudoSession, RefineJob, SessionConfig};
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
/// Whether this lane refines, and where the work goes when it does.
///
/// Two arrangements rather than one with an `Option` inside it, because a lane that does not refine
/// must behave exactly as it did before this existed — same session, same configuration, no audio
/// retained. `refine_model` is unset for everybody who has not asked for it, and the plain arm is
/// what they keep running.
enum Engine {
    Plain(PseudoSession<Box<dyn Decoder>>),
    /// The fast model, plus a queue of finished utterances for a slower one.
    ///
    /// The queue is shared rather than returned: this is a pipeline stage, and a stage emits
    /// frames. A `RefineJob` is not a frame — nothing downstream of the recogniser wants audio
    /// back — so it goes out of the side, the same way the diarizer's audio tap does.
    Hybrid {
        session: HybridSession<Box<dyn Decoder>>,
        jobs: Arc<Mutex<VecDeque<RefineJob>>>,
    },
}

pub struct Recognise {
    lane: Lane,
    engine: Engine,
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
        Self::from_session(lane, PseudoSession::new(decoder, config))
    }

    /// The same lane, keeping each finished utterance for a second, slower model.
    ///
    /// The jobs it fills are drained by the session runner and executed off the audio thread — a
    /// refine decode is a second or more and this stage is called from the frame loop.
    #[must_use]
    pub fn refining(
        lane: Lane,
        decoder: Box<dyn Decoder>,
        config: SessionConfig,
        jobs: Arc<Mutex<VecDeque<RefineJob>>>,
    ) -> Self {
        Self {
            lane,
            engine: Engine::Hybrid {
                session: HybridSession::new(decoder, config),
                jobs,
            },
            pending: None,
        }
    }

    /// Hand a finished utterance to whoever is draining the queue, if this lane has one.
    fn offer(&self, job: Option<RefineJob>) {
        let (Engine::Hybrid { jobs, .. }, Some(job)) = (&self.engine, job) else {
            return;
        };
        let Ok(mut queue) = jobs.lock() else {
            // A poisoned queue means a drainer panicked. The recording is not the thing that should
            // end for it, and the cost is a sentence that keeps the fast model's text.
            return;
        };
        // Bounded, and the *oldest* goes. Past this the refine model is losing to the speaker, and
        // the same argument the live translator makes applies: a revision arriving three minutes
        // after the line it corrects is worse than the line standing.
        if queue.len() >= MAX_PENDING_REFINEMENTS {
            queue.pop_front();
        }
        queue.push_back(job);
    }

    /// Wrap a session somebody already built.
    ///
    /// The comparison path needs this: to prove the chain and the hand-written loop agree, both
    /// have to run the *same* session with the same configuration, not two sessions that happen to
    /// be configured alike.
    #[must_use]
    pub fn from_session(lane: Lane, session: PseudoSession<Box<dyn Decoder>>) -> Self {
        Self {
            lane,
            engine: Engine::Plain(session),
            pending: None,
        }
    }

    #[must_use]
    pub fn decode_count(&self) -> u64 {
        match &self.engine {
            Engine::Plain(session) => session.decode_count(),
            Engine::Hybrid { session, .. } => session.decode_count(),
        }
    }

    #[must_use]
    pub fn suppressed_count(&self) -> u64 {
        match &self.engine {
            Engine::Plain(session) => session.suppressed_count(),
            Engine::Hybrid { session, .. } => session.suppressed_count(),
        }
    }

    #[must_use]
    pub fn is_speaking(&self) -> bool {
        match &self.engine {
            Engine::Plain(session) => session.is_speaking(),
            Engine::Hybrid { session, .. } => session.is_speaking(),
        }
    }

    /// Where the meeting has got to, so a rebuilt lane carries on rather than starting over.
    #[must_use]
    pub fn position(&self) -> (u64, usize) {
        match &self.engine {
            Engine::Plain(session) => session.position(),
            Engine::Hybrid { session, .. } => session.position(),
        }
    }

    pub fn resume_at(&mut self, seq: u64, samples_seen: usize) {
        match &mut self.engine {
            Engine::Plain(session) => session.resume_at(seq, samples_seen),
            Engine::Hybrid { session, .. } => session.resume_at(seq, samples_seen),
        }
    }
}

/// Finished utterances allowed to wait for the refine model.
///
/// Two is deliberate and small. The refine pass exists to improve text the user is already reading;
/// a backlog means it is not keeping up, and every further job makes the revision land further from
/// the moment it would have helped.
const MAX_PENDING_REFINEMENTS: usize = 2;

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
                let events = match &mut self.engine {
                    Engine::Plain(session) => session.accept(&samples, probability)?,
                    Engine::Hybrid { session, .. } => {
                        let out = session.accept(&samples, probability)?;
                        self.offer(out.refine);
                        out.events
                    }
                };
                Ok(events.into_iter().map(Frame::Event).collect())
            }
            Frame::Flush | Frame::End => {
                // The open utterance, before the control frame that ended it — otherwise the last
                // sentence of every meeting arrives after the sink has stopped listening.
                let events = match &mut self.engine {
                    Engine::Plain(session) => session.flush()?,
                    Engine::Hybrid { session, .. } => {
                        let closed = session.flush()?;
                        self.offer(closed.refine);
                        closed.events
                    }
                };
                let mut out: Vec<Frame> = events.into_iter().map(Frame::Event).collect();
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
        Detect::new(
            lane,
            Box::new(LoudIsSpeech {
                width: 160,
                resets: 0,
            }),
        )
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

        assert!(
            !finals.is_empty(),
            "expected a committed utterance: {produced:?}"
        );
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

#[cfg(test)]
mod refining_tests {
    use super::*;
    use summo_core::segment::Lane;

    /// A decoder that names the language it heard, which is what routing needs.
    struct Bilingual {
        language: &'static str,
    }

    impl Decoder for Bilingual {
        fn name(&self) -> &str {
            "bilingual"
        }
        fn decode(&mut self, _samples: &[f32]) -> summo_core::Result<summo_asr::Transcript> {
            Ok(summo_asr::Transcript {
                text: "xin chào".into(),
                language: Some(self.language.to_string()),
                ..summo_asr::Transcript::default()
            })
        }
    }

    fn drive(stage: &mut Recognise, lane: Lane) {
        // Loud, then quiet: the gate opens on speech and commits the utterance on trailing silence.
        for _ in 0..40 {
            let _ = stage
                .push(Frame::audio(lane, vec![0.8; 160], 16_000))
                .unwrap();
            let _ = stage
                .push(Frame::Voice {
                    lane,
                    probability: 0.95,
                })
                .unwrap();
        }
        for _ in 0..80 {
            let _ = stage
                .push(Frame::audio(lane, vec![0.0; 160], 16_000))
                .unwrap();
            let _ = stage
                .push(Frame::Voice {
                    lane,
                    probability: 0.02,
                })
                .unwrap();
        }
        let _ = stage.push(Frame::Flush).unwrap();
    }

    /// A refining lane hands out the finished utterance — its audio, its text, and the language the
    /// fast model reported. All three are needed downstream: the audio to decode again, the text to
    /// beat, and the language to decide whether the second model is the right one at all.
    #[test]
    fn a_refining_lane_offers_the_utterance_it_just_closed() {
        let jobs: Arc<Mutex<VecDeque<RefineJob>>> = Arc::new(Mutex::new(VecDeque::new()));
        let lane = Lane::Mic;
        let mut stage = Recognise::refining(
            lane,
            Box::new(Bilingual { language: "vi" }),
            SessionConfig {
                lane,
                ..SessionConfig::default()
            },
            jobs.clone(),
        );

        drive(&mut stage, lane);

        let queued = jobs.lock().unwrap();
        let job = queued.front().expect("an utterance to refine");
        assert_eq!(job.language.as_deref(), Some("vi"));
        assert_eq!(job.text, "xin chào");
        assert!(!job.pcm.is_empty(), "the audio has to come with it");
    }

    /// And a plain lane offers nothing, which is what keeps every user who has not asked for this
    /// on exactly the pipeline they had: no audio retained, no queue, no second decode.
    #[test]
    fn a_plain_lane_keeps_no_audio_and_queues_nothing() {
        let jobs: Arc<Mutex<VecDeque<RefineJob>>> = Arc::new(Mutex::new(VecDeque::new()));
        let lane = Lane::Mic;
        let mut stage = Recognise::new(
            lane,
            Box::new(Bilingual { language: "vi" }),
            SessionConfig {
                lane,
                ..SessionConfig::default()
            },
        );

        drive(&mut stage, lane);

        assert!(jobs.lock().unwrap().is_empty());
    }

    /// The queue is bounded, and it is the *oldest* that goes. A refine model losing to the speaker
    /// should be revising the line somebody is looking at, not the one three sentences back.
    #[test]
    fn a_backlog_drops_the_stalest_utterance() {
        let jobs: Arc<Mutex<VecDeque<RefineJob>>> = Arc::new(Mutex::new(VecDeque::new()));
        let lane = Lane::Mic;
        let stage = Recognise::refining(
            lane,
            Box::new(Bilingual { language: "vi" }),
            SessionConfig {
                lane,
                ..SessionConfig::default()
            },
            jobs.clone(),
        );

        for seq in 0..(MAX_PENDING_REFINEMENTS + 3) {
            stage.offer(Some(RefineJob {
                seq: seq as u64,
                lane,
                t0: 0.0,
                t1: 1.0,
                pcm: vec![0.1],
                language: Some("vi".into()),
                text: format!("câu {seq}"),
            }));
        }

        let queued = jobs.lock().unwrap();
        assert_eq!(queued.len(), MAX_PENDING_REFINEMENTS);
        assert_eq!(
            queued.front().map(|job| job.seq),
            Some(3),
            "the oldest went, not the newest"
        );
    }
}
