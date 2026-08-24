//! Fast text now, accurate text a moment later.
//!
//! The two model families trade off against each other exactly where the user notices. A small
//! streaming model puts words on screen in under 200 ms but mishears names, numbers and
//! code-switched English. A large batch model gets those right and takes a second or more per
//! utterance. Choosing one means choosing which complaint to receive.
//!
//! [`HybridSession`] declines to choose: the fast model drives the live transcript, and when an
//! utterance closes its audio is handed to the slow model, whose result replaces the text in place
//! as an [`Event::Revise`]. The user reads immediately and the record settles to the better
//! transcript.
//!
//! **The refine pass never runs on the audio thread.** [`HybridSession::accept`] returns a
//! [`RefineJob`] describing work to do; the caller runs it on a worker and feeds the result back.
//! Doing it inline would mean a second of decode inside the callback that is supposed to be
//! consuming microphone frames, and the capture buffer would overrun.

use summo_core::{
    Event, Result,
    segment::{Lane, Segment, SegmentSource},
};

use crate::{
    decoder::Decoder,
    hallucination::HallucinationFilter,
    session::{PseudoSession, SessionConfig},
};

/// A finished utterance waiting for a better transcription.
#[derive(Debug, Clone, PartialEq)]
pub struct RefineJob {
    pub seq: u64,
    pub lane: Lane,
    pub t0: f64,
    pub t1: f64,
    /// The utterance audio, already trimmed by the gate.
    pub pcm: Vec<f32>,
    /// What the fast model said this utterance was in, when it says.
    ///
    /// Carried so the caller can decide whether the refine model is the right one for *this*
    /// sentence rather than for the meeting. That is the whole of bilingual support: a call held
    /// half in Vietnamese and half in English wants an accurate Vietnamese model on the Vietnamese
    /// utterances and nothing extra on the rest, and until this field existed the only choice
    /// available was one model for all of it.
    pub language: Option<String>,
    /// The text the fast model produced, which is what a refinement has to beat.
    pub text: String,
}

/// What one frame produced.
#[derive(Debug, Default)]
pub struct HybridOutput {
    /// Partials and finals from the fast model, ready to display.
    pub events: Vec<Event>,
    /// Work for the refine model, if an utterance just closed. Run it off the audio thread.
    pub refine: Option<RefineJob>,
}

impl HybridOutput {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.refine.is_none()
    }
}

/// A live lane plus deferred refinement.
pub struct HybridSession<D: Decoder> {
    live: PseudoSession<D>,
    lane: Lane,
}

impl<D: Decoder> HybridSession<D> {
    /// Wrap a fast decoder. `cfg.keep_final_pcm` is forced on, since refinement needs the audio.
    #[must_use]
    pub fn new(fast: D, mut cfg: SessionConfig) -> Self {
        cfg.keep_final_pcm = true;
        let lane = cfg.lane;
        Self {
            live: PseudoSession::new(fast, cfg),
            lane,
        }
    }

    /// Utterances the fast model threw away as hallucinated, which a lane reports whichever
    /// arrangement is behind it.
    #[must_use]
    pub fn suppressed_count(&self) -> u64 {
        self.live.suppressed_count()
    }

    /// Feed one frame and its speech probability.
    pub fn accept(&mut self, frame: &[f32], speech_prob: f32) -> Result<HybridOutput> {
        let events = self.live.accept(frame, speech_prob)?;
        Ok(self.attach_refine(events))
    }

    /// Close any open utterance at the end of a session.
    pub fn flush(&mut self) -> Result<HybridOutput> {
        let events = self.live.flush()?;
        Ok(self.attach_refine(events))
    }

    /// Pair a just-emitted final with the audio that produced it.
    fn attach_refine(&mut self, events: Vec<Event>) -> HybridOutput {
        // Only a final carries a refine job, and only one final can close per frame.
        let finished = events.iter().find_map(|e| match e {
            Event::Final(s) => Some((s.seq, s.t0, s.t1, s.text.clone())),
            _ => None,
        });

        // The audio is taken either way: leaving it behind after a suppressed final would hand it
        // to the *next* utterance's job. Same for the language, for the same reason.
        let pcm = self.live.take_final_pcm();
        let language = self.live.take_final_language();

        let refine = match (finished, pcm) {
            (Some((seq, t0, t1, text)), Some(pcm)) => Some(RefineJob {
                seq,
                lane: self.lane,
                t0,
                t1,
                pcm,
                language,
                text,
            }),
            _ => None,
        };

        HybridOutput { events, refine }
    }

    /// Run a refine job and turn a better transcript into a [`Event::Revise`].
    ///
    /// Returns `None` when the refined text adds nothing — identical to what is already displayed,
    /// empty, or judged a hallucination. Rewriting a line with the same words would make the
    /// transcript flicker for no reason.
    pub fn refine(
        job: &RefineJob,
        decoder: &mut dyn Decoder,
        filter: &HallucinationFilter,
        current_text: &str,
    ) -> Result<Option<Event>> {
        let transcript = decoder.decode(&job.pcm)?;
        decoder.reset();

        if !filter.judge(&transcript).is_keep() || transcript.is_empty() {
            return Ok(None);
        }
        if transcript.text.trim() == current_text.trim() {
            return Ok(None);
        }

        let mut segment = Segment::new(job.seq, job.lane, transcript.text, job.t0, job.t1);
        segment.source = SegmentSource::Revised;
        segment.conf = transcript.confidence;
        segment.words = transcript.words;
        Ok(Some(Event::Revise(segment)))
    }

    /// The hallucination policy in force, so refinement is judged by the same rules.
    #[must_use]
    pub fn filter(&self) -> &HallucinationFilter {
        self.live.filter()
    }

    #[must_use]
    pub fn decode_count(&self) -> u64 {
        self.live.decode_count()
    }

    #[must_use]
    pub fn is_speaking(&self) -> bool {
        self.live.is_speaking()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{
        Transcript,
        test_support::{FixedDecoder, GrowingDecoder},
    };

    const FRAME: usize = 160;

    fn run<D: Decoder>(
        session: &mut HybridSession<D>,
        script: &[(bool, usize)],
    ) -> (Vec<Event>, Vec<RefineJob>) {
        let speech = vec![0.5_f32; FRAME];
        let quiet = vec![0.0_f32; FRAME];
        let (mut events, mut jobs) = (Vec::new(), Vec::new());
        for &(is_speech, count) in script {
            for _ in 0..count {
                let (frame, prob) = if is_speech {
                    (&speech, 0.9)
                } else {
                    (&quiet, 0.01)
                };
                let out = session.accept(frame, prob).unwrap();
                events.extend(out.events);
                jobs.extend(out.refine);
            }
        }
        (events, jobs)
    }

    #[test]
    fn a_closed_utterance_produces_refine_work_carrying_its_audio() {
        let mut s = HybridSession::new(
            FixedDecoder::new("toi nghi minh nen"),
            SessionConfig::default(),
        );
        let (events, jobs) = run(&mut s, &[(true, 100), (false, 60)]);

        assert_eq!(jobs.len(), 1, "one utterance, one refine job");
        assert!(
            !jobs[0].pcm.is_empty(),
            "the job must carry the audio to re-decode"
        );

        let Event::Final(seg) = events
            .iter()
            .find(|e| matches!(e, Event::Final(_)))
            .unwrap()
        else {
            unreachable!()
        };
        assert_eq!(
            jobs[0].seq, seg.seq,
            "refine must target the segment it came from"
        );
        assert_eq!(jobs[0].t0, seg.t0);
    }

    #[test]
    fn refinement_replaces_the_text_in_place() {
        let mut s = HybridSession::new(
            FixedDecoder::new("toi nghi minh nen"),
            SessionConfig::default(),
        );
        let (_, jobs) = run(&mut s, &[(true, 100), (false, 60)]);

        let mut slow = FixedDecoder::new("Tôi nghĩ mình nên");
        let event = HybridSession::<FixedDecoder>::refine(
            &jobs[0],
            &mut slow,
            s.filter(),
            "toi nghi minh nen",
        )
        .unwrap()
        .expect("a better transcript should produce a revision");

        let Event::Revise(seg) = event else {
            panic!("expected a revise event")
        };
        assert_eq!(seg.text, "Tôi nghĩ mình nên");
        assert_eq!(
            seg.seq, jobs[0].seq,
            "the revision must land on the same segment"
        );
        assert_eq!(seg.source, SegmentSource::Revised);
    }

    #[test]
    fn identical_refined_text_does_not_cause_a_flicker() {
        let mut s = HybridSession::new(FixedDecoder::new("đã đúng rồi"), SessionConfig::default());
        let (_, jobs) = run(&mut s, &[(true, 100), (false, 60)]);

        let mut slow = FixedDecoder::new("đã đúng rồi");
        let event =
            HybridSession::<FixedDecoder>::refine(&jobs[0], &mut slow, s.filter(), "đã đúng rồi")
                .unwrap();

        assert!(
            event.is_none(),
            "an unchanged transcript must not be rewritten"
        );
    }

    #[test]
    fn a_hallucinating_refine_model_cannot_corrupt_good_text() {
        let mut s =
            HybridSession::new(FixedDecoder::new("nội dung thật"), SessionConfig::default());
        let (_, jobs) = run(&mut s, &[(true, 100), (false, 60)]);

        struct Hallucinator;
        impl Decoder for Hallucinator {
            fn decode(&mut self, _pcm: &[f32]) -> Result<Transcript> {
                Ok(Transcript {
                    text: "Thank you.".into(),
                    no_speech_prob: Some(0.85),
                    ..Transcript::default()
                })
            }
            fn name(&self) -> &str {
                "hallucinator"
            }
        }

        let event = HybridSession::<FixedDecoder>::refine(
            &jobs[0],
            &mut Hallucinator,
            s.filter(),
            "nội dung thật",
        )
        .unwrap();

        assert!(
            event.is_none(),
            "refinement must be filtered like any other output"
        );
    }

    #[test]
    fn live_partials_still_flow_while_refinement_is_pending() {
        let mut s = HybridSession::new(
            GrowingDecoder::new("một hai ba bốn năm"),
            SessionConfig::default(),
        );
        let (events, jobs) = run(&mut s, &[(true, 100), (false, 60)]);

        let partials = events
            .iter()
            .filter(|e| matches!(e, Event::Partial(_)))
            .count();
        assert!(
            partials >= 3,
            "the fast lane must keep typing, got {partials} partials"
        );
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn a_suppressed_final_leaves_no_stale_audio_for_the_next_utterance() {
        // The fast model produces only boilerplate, so no final is emitted. The audio must still be
        // consumed, or the next utterance's refine job would carry the wrong recording.
        struct Boilerplate;
        impl Decoder for Boilerplate {
            fn decode(&mut self, _pcm: &[f32]) -> Result<Transcript> {
                Ok(Transcript {
                    text: "Thank you.".into(),
                    no_speech_prob: Some(0.9),
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

        let mut s = HybridSession::new(Boilerplate, SessionConfig::default());
        let (events, jobs) = run(
            &mut s,
            &[(true, 100), (false, 60), (true, 100), (false, 60)],
        );

        assert!(events.is_empty(), "nothing should have been displayed");
        assert!(jobs.is_empty(), "no final means no refine job");
    }

    #[test]
    fn several_utterances_produce_one_job_each_in_order() {
        let mut s = HybridSession::new(FixedDecoder::new("câu"), SessionConfig::default());
        let (_, jobs) = run(
            &mut s,
            &[
                (true, 60),
                (false, 60),
                (true, 60),
                (false, 60),
                (true, 60),
                (false, 60),
            ],
        );

        assert_eq!(jobs.len(), 3);
        let seqs: Vec<u64> = jobs.iter().map(|j| j.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[test]
    fn flush_also_yields_refine_work() {
        let mut s = HybridSession::new(FixedDecoder::new("chưa xong"), SessionConfig::default());
        run(&mut s, &[(true, 100)]);

        let out = s.flush().unwrap();
        assert_eq!(out.events.len(), 1);
        assert!(
            out.refine.is_some(),
            "a flushed utterance still deserves refinement"
        );
    }
}
