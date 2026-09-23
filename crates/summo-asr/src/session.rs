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
//! rail is the CPU budget, not the multiplier: the cadence is derived from the decoder's *measured*
//! speed and from how long the open utterance already is, so a heavier model — or a longer sentence
//! — refreshes less often instead of falling behind. See [`SessionConfig::partial_cpu_budget`],
//! which is where a long sentence used to take the whole thing past real time.
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
    denoise::Denoiser,
    hallucination::{HallucinationFilter, Verdict},
};

/// How a session drives its decoder.
#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    pub gate: GateConfig,
    /// Least audio added between partial re-decodes. Smaller means more responsive text and more
    /// CPU; the useful range is roughly 100–400 ms.
    ///
    /// A floor, not the cadence. See [`SessionConfig::partial_cpu_budget`].
    pub partial_step_ms: u32,
    /// Share of one core the partial re-decodes are allowed while somebody is speaking.
    ///
    /// The cadence is derived from this and from the decoder's *measured* speed, because the cost
    /// of a partial is not constant: each one re-decodes the whole open utterance, so it grows
    /// with the sentence while a fixed cadence asks for them just as often. Measured on this
    /// machine, same audio, same model, the only difference being where the speaker paused:
    ///
    /// | Utterances | Decodes | Real-time factor |
    /// | --- | --- | --- |
    /// | eight short ones, 28.8 s | 129 | 0.27 |
    /// | one of 22 s, 27.4 s | 172 | **1.32** |
    ///
    /// A model whose own real-time factor is 0.017, made seventy-eight times slower than itself by
    /// nobody pausing — and past 1.0 it cannot keep up live at all, which is a transcript that
    /// stops arriving in the middle of a long sentence. Exactly when somebody is saying the thing
    /// worth transcribing.
    ///
    /// So: `step = rtf × open_seconds ÷ budget`. Cost per second of speech stays flat, a long
    /// sentence refreshes less often instead of falling behind, and a slow model or a busy machine
    /// degrades the same way rather than stalling.
    ///
    /// 0.35 of a core, which leaves room for the detector, the final decode, and a second model.
    pub partial_cpu_budget: f32,
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
            partial_cpu_budget: 0.35,
            lane: Lane::Mic,
            emit_partials: true,
            keep_final_pcm: false,
        }
    }
}

/// Longest a speaker goes without the text catching up.
///
/// The cadence grows with the sentence, and something has to stop it growing without end. Two
/// seconds is the point where "typing along with you" becomes "answering later": past it the
/// screen reads as stuck even though the words are still coming.
///
/// Reached only by a very long sentence or a very slow model, and reaching it is the honest
/// outcome — the alternative is a decoder that is further behind every second.
const MAX_PARTIAL_STEP_MS: u32 = 2_000;

/// Tail a partial will re-decode before it freezes what it already has.
///
/// The cadence fix held the *cost per second of speech* still, which stopped a long sentence
/// taking the machine past real time — but each individual decode still grew, so the text got
/// coarser the longer somebody talked and a single decode of a thirty-second sentence still blocked
/// the audio thread for half of it.
///
/// The cost has no reason to grow at all. A partial re-decodes the whole open utterance, and almost
/// all of that utterance is not going to change: the words spoken ten seconds ago are settled. So
/// once the undecided tail passes this, the text so far is frozen and every later partial decodes
/// only what has been said since. Six seconds of tail is a fixed price, whether the sentence is ten
/// seconds long or ten minutes.
///
/// The frozen text is a *partial*, and partials are cosmetic — the final decodes the whole
/// utterance in one piece, as it always did, and replaces all of it.
const COMMIT_AFTER_MS: u32 = 6_000;

/// And the point past which it freezes whether or not there is a good place to.
///
/// Freezing between words needs a pause, and somebody reading aloud can go a long time without one.
/// Past this the seam is cut mid-word, which costs a word in text that is about to be replaced
/// anyway — cheaper than a decode that is still growing at twelve seconds.
const COMMIT_LATEST_MS: u32 = 12_000;

/// Quiet that makes a moment "between words" rather than inside one.
///
/// Far below [`GateConfig::min_silence_s`], which is the quiet that ends a sentence. This is the
/// gap between two words, and cutting there is what keeps a frozen prefix from ending mid-syllable.
const DIP_MS: u32 = 160;

/// How much a new measurement moves the running estimate.
///
/// Slow, because the thing being estimated barely changes: it is one model on one machine. What
/// does change is the noise — another process taking cores, a thermal step — and a fast filter
/// would swing the cadence around on it.
const RTF_SMOOTHING: f32 = 0.2;

/// Drives one decoder over one audio lane.
pub struct PseudoSession<D: Decoder> {
    decoder: D,
    gate: VadGate,
    cfg: SessionConfig,
    filter: HallucinationFilter,
    /// Samples in the open utterance at the last partial decode.
    last_partial_len: usize,
    /// Text of the part of the open utterance that is no longer being re-decoded.
    ///
    /// See [`COMMIT_AFTER_MS`]. Empty for every utterance short enough that nothing was frozen,
    /// which is nearly all of them.
    committed_text: String,
    /// How much of the open utterance `committed_text` accounts for.
    committed_len: usize,
    /// How fast this decoder is on this machine, as seconds of work per second of audio.
    ///
    /// Measured rather than declared. The registry publishes a real-time factor, but it was taken
    /// on somebody else's machine with the whole box to itself, and the number that decides how
    /// often it is safe to re-decode has to be the one this laptop is getting right now — with
    /// four other models resident, on whatever cores are left.
    ///
    /// `None` until the first decode returns, which is why [`SessionConfig::partial_step_ms`] is
    /// still the floor and still the answer for the first partial of a session.
    decode_rtf: Option<f32>,
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
    /// Speech enhancement, when a model for it is installed and chosen.
    ///
    /// `None` for almost everybody, and that is the intended default — see [`crate::denoise`] on
    /// why a denoiser is not free accuracy. Boxed rather than a second type parameter because the
    /// answer to "is there one" is a runtime setting, and threading `PseudoSession<D, N>` through
    /// `stages.rs` and `HybridSession` to encode a `bool` in the type system would be a large
    /// change to say a small thing.
    denoiser: Option<Box<dyn Denoiser>>,
    /// The language this decoder hears, when it hears exactly one.
    ///
    /// A specialist reports no language per utterance — there is only one, so there is nothing to
    /// report — while a multilingual runtime answers each time. Both have to reach the segment,
    /// because "translate each line into the other language" cannot decide anything without knowing
    /// which language a line is in, and the ordinary bilingual setup is exactly a specialist live
    /// with a broad model behind it.
    ///
    /// Set from the manifest's `langs` when it names one, so this is a fact the registry already
    /// carries rather than a guess about the audio.
    language: Option<String>,
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
            committed_text: String::new(),
            committed_len: 0,
            decode_rtf: None,
            decodes: 0,
            suppressed: 0,
            last_final_pcm: None,
            last_final_language: None,
            denoiser: None,
            language: None,
        }
    }

    /// Clean each finished utterance with this model before decoding it.
    #[must_use]
    pub fn hearing(mut self, language: Option<String>) -> Self {
        self.language = language
            .map(|l| l.trim().to_ascii_lowercase())
            .filter(|l| !l.is_empty());
        self
    }

    pub fn with_denoiser(mut self, denoiser: Option<Box<dyn Denoiser>>) -> Self {
        self.denoiser = denoiser;
        self
    }

    /// The speech enhancer in use, for `/status` and the performance HUD.
    #[must_use]
    pub fn denoiser_name(&self) -> Option<&str> {
        self.denoiser.as_ref().map(|d| d.name())
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
                self.forget_the_open_utterance();
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

    /// How much new audio to wait for, given how long the open utterance already is.
    ///
    /// The whole of the fix described on [`SessionConfig::partial_cpu_budget`]. A partial decodes
    /// everything said so far, so its cost is `rtf × open_seconds`; asking for one every `step`
    /// seconds therefore costs `rtf × open_seconds ÷ step` of a core, continuously, for as long as
    /// the speaker keeps going. Holding that at the budget gives the step directly.
    ///
    /// Before the first decode has been timed there is no `rtf` to use, so the floor stands — which
    /// is right anyway: the first partial of an utterance is the one worth being fastest about, and
    /// it decodes a fraction of a second of audio.
    fn partial_step_ms(&self, open_secs: f64) -> u32 {
        let Some(rtf) = self.decode_rtf else {
            return self.cfg.partial_step_ms;
        };
        let budget = self.cfg.partial_cpu_budget.clamp(0.05, 0.9);
        let step_ms = (rtf * open_secs as f32 / budget * 1000.0) as u32;
        step_ms.clamp(self.cfg.partial_step_ms, MAX_PARTIAL_STEP_MS)
    }

    /// Fold one timed decode into the running estimate of how fast this decoder is here.
    fn observe(&mut self, took: std::time::Duration, audio_secs: f64) {
        if audio_secs <= 0.0 {
            return;
        }
        let measured = took.as_secs_f32() / audio_secs as f32;
        self.decode_rtf = Some(match self.decode_rtf {
            // The first measurement is taken whole. Starting from a guess and filtering towards the
            // truth would spend the first seconds of every recording at the wrong cadence, and the
            // first seconds are the ones somebody is watching to see whether this thing works.
            None => measured,
            Some(current) => current + (measured - current) * RTF_SMOOTHING,
        });
    }

    /// Forget everything known about the utterance that just ended.
    ///
    /// One place, because the two halves of it are easy to separate by accident and the symptom of
    /// separating them is a sentence that begins with the end of the previous one.
    fn forget_the_open_utterance(&mut self) {
        self.last_partial_len = 0;
        self.committed_text.clear();
        self.committed_len = 0;
    }

    /// Re-decode the *undecided tail* of the open utterance if enough new audio has arrived.
    ///
    /// Only the tail. See [`COMMIT_AFTER_MS`]: what was said ten seconds ago is not going to change,
    /// so re-deciding it on every refresh is work with a known answer — and it is the work that made
    /// a long sentence cost more with every second of it.
    fn maybe_partial(&mut self, seq: u64, t0: f64, t1: f64) -> Result<Vec<Event>> {
        if !self.cfg.emit_partials || !self.decoder.supports_partials() {
            return Ok(Vec::new());
        }

        let open_len = self.gate.open_pcm().len();
        // Clamped rather than trusted: the gate owns this buffer and an utterance boundary this
        // function did not see would leave a commit point past the end of a shorter one.
        let committed = self.committed_len.min(open_len);
        let tail_len = open_len - committed;

        // Sized on the tail, because the tail is what the decode will cost.
        let step = ms_to_samples(self.partial_step_ms(samples_to_secs(tail_len)));
        if open_len < self.last_partial_len + step {
            return Ok(Vec::new());
        }
        self.last_partial_len = open_len;

        // The borrow checker cannot see that `decode` does not touch the gate, so copy the window.
        // At a few seconds of 16 kHz mono this is tens of kilobytes — noise next to the decode.
        // Levelled on the way in — see `decoder::levelled`. A quiet microphone is the ordinary
        // recording, not an exotic one, and every model measured reads it better this way.
        let window = crate::decoder::levelled(&self.gate.open_pcm()[committed..]).into_owned();
        let quiet = self.gate.quiet_samples();
        self.decodes += 1;
        let began = std::time::Instant::now();
        let transcript = self.decoder.decode(&window)?;
        self.observe(began.elapsed(), samples_to_secs(window.len()));

        let text = match (self.committed_text.as_str(), transcript.text.trim()) {
            ("", tail) => tail.to_string(),
            (head, "") => head.to_string(),
            (head, tail) => format!("{head} {tail}"),
        };

        // Freeze, once the tail is long enough to be worth not decoding again — at a gap between
        // words if there is one, and regardless once the tail is long enough that waiting for one
        // costs more than cutting badly.
        let dip = quiet >= ms_to_samples(DIP_MS);
        if tail_len >= ms_to_samples(COMMIT_LATEST_MS)
            || (dip && tail_len >= ms_to_samples(COMMIT_AFTER_MS))
        {
            self.committed_text = text.clone();
            self.committed_len = open_len;
            // The decoder has state per call for some runtimes; the next tail is a new utterance as
            // far as it is concerned.
            self.decoder.reset();
        }

        if text.is_empty() {
            return Ok(Vec::new());
        }
        let mut segment = Segment::new(seq, self.cfg.lane, text, t0, t1);
        segment.source = SegmentSource::Partial;
        segment.conf = transcript.confidence;
        segment.language = transcript
            .language
            .clone()
            .or_else(|| self.language.clone());
        Ok(vec![Event::Partial(segment)])
    }

    /// Decode a closed utterance and emit it, unless it looks invented.
    fn finalize(&mut self, seq: u64, t0: f64, t1: f64, pcm: &[f32]) -> Result<Vec<Event>> {
        // Including anything frozen mid-sentence. The final decodes the whole utterance in one
        // piece — it always did — so a seam the partials had to cut does not survive into the
        // transcript, and the next utterance does not inherit the end of this one.
        self.forget_the_open_utterance();

        // Cleaned once, here, and then it *is* the utterance: the decoder sees it, the retained
        // audio is it, and a second model refining this line later refines the same seconds the
        // first one heard. Denoising per consumer would run the model twice and — worse — let the
        // two models disagree about what was said because they were listening to different audio.
        //
        // A failed clean-up is not a lost utterance. Noise costs words; a runtime that could not
        // load or could not run costs nothing if the original is decoded instead, so this warns and
        // carries on rather than propagating. The one thing it must not do is stay quiet: a
        // denoiser that silently never ran is the shape of bug this whole file exists to correct.
        let cleaned = match self.denoiser.as_mut() {
            None => None,
            Some(denoiser) => match denoiser.denoise(pcm) {
                Ok(cleaned) => Some(cleaned),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        seq,
                        model = denoiser.name(),
                        "decoding the original audio; the speech enhancer failed"
                    );
                    None
                }
            },
        };
        let pcm: &[f32] = cleaned.as_deref().unwrap_or(pcm);

        // Levelled once, here, and then it *is* the utterance — the same argument the denoiser
        // above is made on. The second model gets the audio the first one heard, so the two cannot
        // disagree about what was said because one of them was listening to a quieter copy.
        let levelled = crate::decoder::levelled(pcm);
        let pcm: &[f32] = &levelled;

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
        // What the model said it heard, else what it can only have heard. A multilingual runtime
        // answers per utterance; a specialist answers nothing and is the one case where the
        // manifest already knows.
        segment.language = transcript
            .language
            .clone()
            .or_else(|| self.language.clone());
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
        Verdict::Annotation => "annotation",
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

    /// A decoder that remembers how much audio it was handed.
    ///
    /// The only thing the commit window is about, and nothing else can see it: the events carry
    /// text, and text says nothing about how much work produced it.
    struct WindowSpy {
        seen: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
        finals: usize,
        /// The samples of the last decode, for the one test that asks what the model heard rather
        /// than how much of it there was.
        keep: Option<std::sync::Arc<std::sync::Mutex<Vec<f32>>>>,
    }

    impl Decoder for WindowSpy {
        fn decode(&mut self, pcm: &[f32]) -> Result<Transcript> {
            self.seen.lock().unwrap().push(pcm.len());
            if let Some(keep) = &self.keep {
                *keep.lock().unwrap() = pcm.to_vec();
            }
            self.finals += 1;
            Ok(Transcript::new(format!("câu {}", self.finals)))
        }
        fn name(&self) -> &str {
            "window-spy"
        }
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

    /// A denoiser that counts, and marks what it touched so the decoder can prove it got the clean
    /// audio rather than the original.
    struct Counting {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        fail: bool,
    }

    impl Denoiser for Counting {
        fn denoise(&mut self, pcm: &[f32]) -> Result<Vec<f32>> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.fail {
                return Err(summo_core::Error::Other("no".into()));
            }
            Ok(pcm.iter().map(|s| s * 0.5).collect())
        }
        fn name(&self) -> &str {
            "counting"
        }
    }

    fn counting(
        fail: bool,
    ) -> (
        Box<dyn Denoiser>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (
            Box::new(Counting {
                calls: calls.clone(),
                fail,
            }),
            calls,
        )
    }

    /// The placement decision, asserted rather than described.
    ///
    /// Once per finished utterance and never on a partial. A partial re-decodes a growing window
    /// every few hundred milliseconds, so a denoiser on that path would clean the same seconds a
    /// dozen times over for text that the final is about to replace — this run makes several
    /// partials, and exactly one of them is a final.
    #[test]
    fn the_enhancer_runs_once_per_utterance_and_never_on_a_partial() {
        let (denoiser, calls) = counting(false);
        let mut s = PseudoSession::new(
            GrowingDecoder::new("một hai ba"),
            SessionConfig {
                partial_step_ms: 100,
                ..SessionConfig::default()
            },
        )
        .with_denoiser(Some(denoiser));

        let events = run(&mut s, &[(true, 200), (false, 60)]);

        assert!(!partials(&events).is_empty(), "the run made no partials");
        assert_eq!(finals(&events).len(), 1);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "one utterance, one clean-up"
        );
    }

    /// The retained audio is exactly what the first model heard, so a second model refining this
    /// line later hears the same seconds. Two models listening to different audio would disagree
    /// about what was said for a reason no reader could see.
    ///
    /// Asserted against the decoder's own input rather than against an amplitude. It used to check
    /// that the kept samples were quiet, because the stub denoiser halves them and the raw audio was
    /// louder — and levelling made both copies end at the same peak, which is the point of levelling
    /// and left that check asserting nothing about which copy it had.
    #[test]
    fn the_audio_kept_for_a_second_model_is_what_the_first_one_heard() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (denoiser, cleaned) = counting(false);
        let mut s = PseudoSession::new(
            WindowSpy {
                seen: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                finals: 0,
                keep: Some(heard.clone()),
            },
            SessionConfig {
                keep_final_pcm: true,
                emit_partials: false,
                ..SessionConfig::default()
            },
        )
        .with_denoiser(Some(denoiser));

        run(&mut s, &[(true, 200), (false, 60)]);
        assert_eq!(
            cleaned.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the enhancer did not run"
        );

        let kept = s
            .take_final_pcm()
            .expect("the utterance should be retained");
        let decoded = heard.lock().unwrap().clone();
        assert_eq!(kept, decoded, "the second model would hear different audio");
    }

    /// Noise costs words. A runtime that could not run costs nothing, as long as the original is
    /// decoded instead — so a failing enhancer must not take the utterance down with it.
    #[test]
    fn an_enhancer_that_fails_does_not_cost_the_utterance() {
        let (denoiser, calls) = counting(true);
        let mut s = PseudoSession::new(FixedDecoder::new("xin chào"), SessionConfig::default())
            .with_denoiser(Some(denoiser));

        let events = run(&mut s, &[(true, 200), (false, 60)]);

        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            finals(&events).len(),
            1,
            "the line was lost because the clean-up failed"
        );
    }

    /// A session that has not timed a decode yet still answers, and answers fast.
    ///
    /// The first partial of a recording is the one somebody is watching to decide whether this
    /// thing works, and it decodes a fraction of a second of audio. Waiting for an estimate before
    /// producing it would spend that moment being careful about nothing.
    #[test]
    fn the_first_partial_does_not_wait_for_a_measurement() {
        let s = PseudoSession::new(FixedDecoder::new("một"), SessionConfig::default());
        assert_eq!(s.partial_step_ms(0.5), 150);
        assert_eq!(s.partial_step_ms(30.0), 150);
    }

    /// The bug, as a unit.
    ///
    /// A partial decodes the whole open utterance, so the cost of one grows with the sentence while
    /// a fixed cadence keeps asking at the same rate. Measured: the same audio and the same model
    /// went from a real-time factor of 0.27 to **1.32** — past real time, unable to keep up — for
    /// no reason except that the speaker did not pause for twenty-two seconds.
    #[test]
    fn a_longer_sentence_is_refreshed_less_often() {
        let mut s = PseudoSession::new(FixedDecoder::new("một"), SessionConfig::default());
        // One second of audio that took twenty milliseconds: rtf 0.02, a typical transducer.
        s.observe(std::time::Duration::from_millis(20), 1.0);

        let short = s.partial_step_ms(2.0);
        let long = s.partial_step_ms(20.0);
        assert!(
            long > short,
            "a long sentence must refresh less often: {long} vs {short}"
        );

        // And the cost is what is actually being held still: `rtf × open ÷ step` of a core, at
        // every length, is the budget and not a multiple of it.
        for open in [2.0_f64, 5.0, 10.0, 20.0] {
            let step = f64::from(s.partial_step_ms(open)) / 1000.0;
            let share = 0.02 * open / step;
            assert!(
                share <= 0.36,
                "at {open}s the partials would take {share:.2} of a core"
            );
        }
    }

    /// The architecture, rather than the mitigation.
    ///
    /// Refreshing less often held the cost per second of speech still; it did not stop each
    /// individual decode from growing. This asserts the thing that does: on a sentence far longer
    /// than the commit window, the audio handed to the decoder stops growing — so a ten-minute
    /// monologue costs the same per refresh as a ten-second one.
    #[test]
    fn a_partial_never_decodes_more_than_the_commit_window() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut s = PseudoSession::new(
            WindowSpy {
                seen: seen.clone(),
                finals: 0,
                keep: None,
            },
            SessionConfig::default(),
        );

        // Twenty-five seconds of unbroken speech, and no silence after it: the utterance stays
        // open, so every decode recorded here is a partial. The *final* decodes the whole thing in
        // one piece and always did — it happens once, and it is not what grows with every refresh.
        run(&mut s, &[(true, 2_500)]);

        let windows: Vec<usize> = seen.lock().unwrap().clone();
        let biggest = windows.iter().copied().max().unwrap_or(0);
        let ceiling = ms_to_samples(COMMIT_LATEST_MS) + ms_to_samples(2_000);
        assert!(
            biggest <= ceiling,
            "a partial decoded {biggest} samples; the window is meant to stop at {ceiling}"
        );
    }

    /// And what was frozen is still in front of the reader.
    ///
    /// Freezing a prefix is only acceptable because the text keeps growing: if the committed half
    /// were dropped, a long sentence would appear to restart every few seconds, which is a worse
    /// screen than the slow one this replaces.
    #[test]
    fn a_frozen_prefix_stays_in_the_partial_text() {
        let mut s = PseudoSession::new(
            FixedDecoder::new("một câu"),
            SessionConfig {
                partial_step_ms: 1_000,
                ..SessionConfig::default()
            },
        );
        let events = run(&mut s, &[(true, 2_500), (false, 60)]);
        let texts = partials(&events);
        let longest = texts.iter().map(|t| t.len()).max().unwrap_or(0);
        assert!(
            longest > "một câu".len(),
            "the partial never grew past one decode's worth: {texts:?}"
        );
        for pair in texts.windows(2) {
            assert!(
                pair[1].len() >= pair[0].len(),
                "the text went backwards: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// Two seconds is where "typing along with you" ends, so the cadence stops growing there even
    /// though holding the budget would say otherwise. A screen that has not moved for five seconds
    /// reads as broken whatever the arithmetic says.
    #[test]
    fn the_cadence_stops_growing_before_the_screen_looks_stuck() {
        let mut s = PseudoSession::new(FixedDecoder::new("một"), SessionConfig::default());
        s.observe(std::time::Duration::from_millis(600), 1.0); // a very slow model
        assert_eq!(s.partial_step_ms(60.0), MAX_PARTIAL_STEP_MS);
    }

    /// And the estimate is of this machine, now. The first decode is taken whole so the cadence is
    /// right immediately; later ones move it slowly, because what changes between them is noise.
    #[test]
    fn the_first_measurement_is_taken_whole_and_the_rest_are_filtered() {
        let mut s = PseudoSession::new(FixedDecoder::new("một"), SessionConfig::default());
        s.observe(std::time::Duration::from_millis(100), 1.0);
        assert!((s.decode_rtf.unwrap() - 0.1).abs() < 1e-6);

        s.observe(std::time::Duration::from_millis(200), 1.0);
        let moved = s.decode_rtf.unwrap();
        assert!(
            moved > 0.1 && moved < 0.2,
            "one sample should not take over the estimate: {moved}"
        );
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
