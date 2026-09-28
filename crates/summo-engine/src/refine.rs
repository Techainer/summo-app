//! A second model, on the sentences it is the right model for.
//!
//! `refine_model` has been a setting since the models screen had a button for it, and until now it
//! did nothing at all in a live recording: the runner built one decoder per lane and there was no
//! second pass. The machinery for the pass — [`summo_asr::HybridSession`] — was written, tested and
//! exported, and never constructed. This is the wiring, and one decision on top of it.
//!
//! ## The decision: per utterance, not per meeting
//!
//! Refining every utterance with the same model is the obvious reading of the setting and the wrong
//! one for the meeting people actually hold. A Vietnamese standup with an English customer on the
//! call is decoded live by a multilingual model — Whisper hears both, badly — and the model that
//! would fix the Vietnamese half is Gipformer, which hears nothing else. Run it on everything and
//! the English sentences come back as Vietnamese-shaped noise, worse than what they replace.
//!
//! So a job is refined only when the refine model *claims the language the live model just heard*.
//! Whisper reports that language per utterance and always has; nothing was reading it. The result
//! is a bilingual meeting transcribed by two models at once — accurate Vietnamese where Vietnamese
//! was spoken, the multilingual model's own text everywhere else — at the cost of one extra decode
//! on the half that benefits, rather than two decodes of everything.
//!
//! An utterance whose language nobody reported is refined. `None` means the live decoder was told
//! its language rather than asked to detect one, which is the single-language case: the user named
//! the language, both models are for it, and skipping would turn the setting off for exactly the
//! people who configured it most deliberately.
//!
//! ## More than one specialist, because a meeting has more than two languages
//!
//! This held exactly one decoder, and `pick_pair` chose it for `languages[0]`. Name Vietnamese,
//! English and Japanese and the Japanese half of the meeting had no specialist at all: it was
//! whatever the multilingual model heard, which for `sense-voice-small` against `whisper-tiny` is
//! 8.4 % character error against 39.8 %. The feature the user asked for — *"tam ngữ cũng được chứ
//! cần gì song ngữ"* — was one `Option` away from existing, and the `Option` was the reason it did
//! not.
//!
//! So a refiner holds a *list* of passes and routes each utterance to the one that names its
//! language. A pass that names a language beats a pass that claims all of them, which is the same
//! rule [`Refiner::wants`] already applied between two models, applied between several.
//!
//! Each pass has its own decoder and so its own in-flight count. One Vietnamese sentence being
//! re-heard no longer blocks the Japanese one behind it — they are different models and there was
//! never a reason beyond the single mutex for them to wait on each other.
//!
//! ## Where the work runs
//!
//! Not on the audio thread. A refine decode is a second or more and the frame loop has 30 ms, so
//! [`Refiner::dispatch`] hands each job to `spawn_blocking` and the revision comes back through a
//! channel as an [`Event::Revise`] on a later frame — the same arrangement live translation uses,
//! for the same reason.

use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use summo_asr::{Decoder, HallucinationFilter, HybridSession, RefineJob};
use summo_core::Event;

/// Jobs allowed to be running at once **per pass**.
///
/// One. The decoder holds mutable inference state and cannot be shared, so a second concurrent job
/// would only wait on the mutex — with the difference that it would hold a blocking thread while it
/// waited. Jobs past this are dropped by the queue in `stages.rs`, which is the right answer: a
/// refine model that cannot keep up is not improving the transcript anybody is reading.
///
/// Per pass rather than per refiner, since passes are separate decoders holding separate state. A
/// global count would have made a Japanese sentence wait on a Vietnamese one for no reason but the
/// counter.
const MAX_IN_FLIGHT: usize = 1;

/// One model, and the languages it was brought in for.
struct Pass {
    /// Named so a log line says which model disagreed, which with several of them is the whole
    /// question.
    id: String,
    /// Behind a mutex because the work happens on a pool thread and a decoder is not `Sync`.
    decoder: Arc<Mutex<Box<dyn Decoder>>>,
    /// The languages this model's manifest claims. Empty means "no claim on record".
    claims: Vec<String>,
    /// The one language this pass was *built* to hear, when it was built for one.
    ///
    /// For a specialist this is its sole claim. For a multilingual model it is the language it was
    /// **forced** to, which is a different thing from what it claims: `whisper-tiny` claims every
    /// language and, told `en`, decodes as English and nothing else.
    ///
    /// That distinction is the whole of this release. Whisper's `language` is not a label, it is
    /// the decoder's prompt: left empty it detects across ninety-nine languages and then decodes
    /// as whichever it picked, so a Vietnamese sentence it guessed was Chinese comes back *in
    /// Chinese characters*. No amount of correcting the label afterwards un-writes the text.
    /// Reported exactly that way: "lọc ra ok nhưng mà không đúng, phải align whisper bắt vi+en?".
    ///
    /// sherpa-onnx takes one language string and offers no way to say "only these". So a language
    /// nothing specialises in gets the general model *pinned to it*, and the arbitration that
    /// already existed picks between the results.
    speaks: Option<String>,
    /// Whether [`Pass::speaks`] came from pinning rather than from the manifest.
    ///
    /// Only for naming. A specialist's id already says which language it is for; a general model
    /// pinned twice is two passes under one id, and a list reading `whisper-tiny, whisper-tiny`
    /// would explain nothing.
    pinned: bool,
    running: Arc<AtomicUsize>,
}

impl Pass {
    /// Whether this model names the language rather than claiming every language.
    ///
    /// The difference decides which pass gets the utterance. `whisper-base` claims `*` and is
    /// 44.2 % character error on Vietnamese; `gipformer-65m` claims `vi` and is a twentieth of
    /// that. Both "want" a Vietnamese sentence under the rules below, and only one of them should
    /// get it.
    fn names(&self, code: &str) -> bool {
        match &self.speaks {
            // Built for one language, whether by specialising in it or by being pinned to it.
            Some(only) => summo_models::langs_cover(std::slice::from_ref(only), code),
            None => {
                !self.claims.iter().any(|l| l == "*")
                    && summo_models::langs_cover(&self.claims, code)
            }
        }
    }

    /// The single language this model speaks, when it speaks exactly one.
    ///
    /// `None` for a multilingual model, which answers for itself per utterance and needs no help,
    /// and `None` for a manifest claiming several — `sense-voice-small` covers five, so which one
    /// a given sentence was is a question only its own output can answer.
    fn sole(&self) -> Option<String> {
        self.speaks.clone()
    }
}

/// One model about to be asked about one utterance.
///
/// Named rather than a tuple because it is built on the socket task and read on a pool thread,
/// and four anonymous fields crossing that boundary is the kind of thing that gets reordered
/// silently.
struct Attempt {
    model: String,
    decoder: Arc<Mutex<Box<dyn Decoder>>>,
    /// The single language this model was built to hear, when it was built for one.
    speaks: Option<String>,
    running: Arc<AtomicUsize>,
}

/// The slower models, and what each is worth running on.
pub struct Refiner {
    /// In the order they were added. The first is the one the session names as *the* second model;
    /// the rest are the specialists the other named languages asked for.
    passes: Vec<Pass>,
    /// The languages the **live** model claims, which is the other half of the decision.
    ///
    /// Without it this could only ask "may the second model attempt this language", and the
    /// answer for a Whisper is always yes — it claims `*`. What it has to ask is "is the second
    /// model *better placed* than the one that already heard it", and that is a question about
    /// both. See [`Refiner::wants`].
    live_claims: Vec<String>,
    /// The languages the *user* said this meeting is in.
    ///
    /// The half that was missing, and the reason a real meeting came back as Chinese. Routing
    /// asked the fast model which language each sentence was and refused every specialist that did
    /// not claim the answer. On a Vietnamese-and-English call `whisper-tiny` answered `zh`, `he`
    /// and `ja`; no specialist claims those, so the rescue path was locked shut by exactly the
    /// guess it exists to correct, and the transcript kept `大家看今天今天今天…`.
    ///
    /// A declared language is a fact the user gave us. A per-utterance label is a guess that is
    /// worthless under a second — zero out of twenty-five for Vietnamese, see `docs/benchmarks.md`
    /// — and, as that meeting showed, wrong well above it too. So the declaration decides *who
    /// runs* and the guess only decides *who runs first*.
    ///
    /// Empty means nobody declared anything, and then the old rule stands: there is nothing better
    /// than the label to go on.
    declared: Vec<String>,
    filter: HallucinationFilter,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
    /// Languages already reported as outside this model's claim.
    ///
    /// So the notice below is said once rather than once per sentence. The per-utterance skip stays
    /// at `debug` for the reason given at its call site — a stream of them during a fast
    /// conversation is not news — but *that the pairing does nothing at all* is, and it was the one
    /// thing nobody could see: a refine model that never runs looks exactly like one that runs and
    /// agrees. `/status` names it either way.
    told: Mutex<BTreeSet<String>>,
    /// Utterances that reached no model because every candidate was busy.
    missed: Arc<AtomicUsize>,
}

impl Refiner {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        decoder: Box<dyn Decoder>,
        claims: Vec<String>,
        live_claims: Vec<String>,
        filter: HallucinationFilter,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            passes: vec![Self::pass(id, decoder, claims)],
            live_claims: live_claims.into_iter().map(|l| l.to_lowercase()).collect(),
            declared: Vec::new(),
            missed: Arc::new(AtomicUsize::new(0)),
            filter,
            tx,
            rx,
            told: Mutex::new(BTreeSet::new()),
        }
    }

    fn pass(id: impl Into<String>, decoder: Box<dyn Decoder>, claims: Vec<String>) -> Pass {
        Self::pinned(id, decoder, claims, None)
    }

    /// A pass, optionally pinned to one language.
    fn pinned(
        id: impl Into<String>,
        decoder: Box<dyn Decoder>,
        claims: Vec<String>,
        forced: Option<String>,
    ) -> Pass {
        let claims: Vec<String> = claims.into_iter().map(|l| l.to_lowercase()).collect();
        // A model pinned to a language speaks that one. Otherwise, a model claiming exactly one
        // language speaks it — which is the specialist case and how this behaved before pinning
        // existed.
        let pinned = forced.is_some();
        let speaks = forced
            .map(|l| l.trim().to_lowercase())
            .or_else(|| match claims.as_slice() {
                [only] if only != "*" => Some(only.clone()),
                _ => None,
            });
        Pass {
            id: id.into(),
            decoder: Arc::new(Mutex::new(decoder)),
            claims,
            speaks,
            pinned,
            running: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Add a specialist for another of the meeting's languages.
    ///
    /// Ignored when a pass already claims the same set, which is how the caller may ask for a
    /// specialist per named language without first working out that two of the names resolve to
    /// the same model — `vi` and `vi-VN` do, and so does any language whose best installed model is
    /// the one already loaded. Loading it twice is several hundred megabytes for nothing.
    pub fn also(&mut self, id: impl Into<String>, decoder: Box<dyn Decoder>, claims: Vec<String>) {
        self.add(Self::pass(id, decoder, claims));
    }

    /// Add the general model, pinned to one language.
    ///
    /// For a declared language nothing specialises in. `whisper-tiny` told `en` decodes English
    /// and only English — it can no longer wander into Chinese on a sentence it mishears, which is
    /// what it did on the meeting that produced this. The result competes with every other pass on
    /// the same terms: whichever output survives `too_quiet_to_be_this_language` wins.
    pub fn pinned_to(
        &mut self,
        id: impl Into<String>,
        language: &str,
        decoder: Box<dyn Decoder>,
        claims: Vec<String>,
    ) {
        self.add(Self::pinned(
            id,
            decoder,
            claims,
            Some(language.to_string()),
        ));
    }

    /// Keep a pass unless an identical one is already held.
    ///
    /// Identity is the model **and** the language it was built for, not the model alone. The same
    /// `whisper-tiny` pinned to `en` and pinned to `ja` are two different decoders answering two
    /// different questions, and keying on the id would have silently kept only the first — which
    /// is the shape of bug this file has now fixed twice.
    fn add(&mut self, pass: Pass) {
        if self
            .passes
            .iter()
            .any(|held| held.id == pass.id && held.speaks == pass.speaks)
        {
            return;
        }
        self.passes.push(pass);
    }

    /// Swap the model the session names as its second one, keeping the extra specialists.
    ///
    /// Chosen in front of the meeting on the models screen. Replacing the whole refiner would drop
    /// the specialists the other named languages are relying on, which is a silent accuracy loss
    /// on the half of the meeting the user was not thinking about when they changed it.
    pub fn replace_primary(
        &mut self,
        id: impl Into<String>,
        decoder: Box<dyn Decoder>,
        claims: Vec<String>,
    ) {
        let pass = Self::pass(id, decoder, claims);
        if self.passes.is_empty() {
            self.passes.push(pass);
        } else {
            self.passes[0] = pass;
        }
    }

    /// Say which languages the user declared this meeting to be in.
    ///
    /// Separate from [`Self::new`] because the declaration belongs to the session and the models
    /// belong to the pairing, and every existing test describes the pairing.
    pub fn declared(&mut self, languages: &[String]) {
        self.declared = languages
            .iter()
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty())
            .collect();
    }

    /// Whether some pass is already built to hear this language.
    ///
    /// Asked before pinning the general model to a declared language: a language with a specialist
    /// has a better model than a pinned Whisper already, and loading both would be a few hundred
    /// megabytes spent to lose a comparison.
    #[must_use]
    pub fn hears(&self, code: &str) -> bool {
        self.passes.iter().any(|pass| pass.names(code))
    }

    /// The models doing the refining, in order. For `/status`, which named only the first.
    #[must_use]
    pub fn models(&self) -> Vec<String> {
        self.passes
            .iter()
            .map(|p| match (&p.speaks, p.pinned) {
                // Named with the language only when it was pinned: the same model pinned twice is
                // two passes under one id, and a list reading `whisper-tiny, whisper-tiny` would
                // explain nothing. A specialist's id already says what it is for.
                (Some(code), true) => format!("{}[{code}]", p.id),
                _ => p.id.clone(),
            })
            .collect()
    }

    /// Whether this model is the right one for what was just heard.
    ///
    /// Two rules, because a pairing has two directions and only one of them was ever expressed.
    ///
    /// ## A specialist heard it, and the second model hears everything
    ///
    /// Gipformer live, Whisper second. The pairing exists for one reason, and
    /// `summo_models::second_opinion` states it: *"covers the rest, so a sentence in another
    /// language is still words."* The rest. Not the Vietnamese the specialist was chosen for.
    ///
    /// The old rule could not express that. It asked only whether the *second* model claims the
    /// language, and Whisper claims `*`, so the answer was always yes — Whisper re-decoded every
    /// Vietnamese utterance and **replaced** text from a model that hears Vietnamese far better.
    /// Measured on FLEURS: whisper-base is 44.2 % CER on Vietnamese. What a user saw was their own
    /// language coming back as English fragments and repeated single words.
    ///
    /// Worse, the case that silently did the most damage is the one with *no* reported language.
    /// A single-language model never reports one — there is nothing to report — so "an unreported
    /// language is refined" handed Whisper the entire meeting.
    ///
    /// So when the live model is a specialist and this one is general: refine only what the
    /// specialist does not cover, and treat an unreported language as the specialist's own,
    /// because for a model that hears one language that is what it is.
    ///
    /// ## Everything else
    ///
    /// Unchanged, and deliberately so: Whisper live with Gipformer second is the arrangement
    /// `e2e/bilingual.mjs` drives, where the specialist's own claim is already the right gate —
    /// it revises the Vietnamese and leaves the English alone.
    ///
    /// A manifest with no languages at all is a claim on everything: that is a gap in the registry
    /// entry rather than a statement that the model is good for nothing, and here the gap is
    /// usually not even in the file — [`Refiner::new`]'s callers fall back to an empty list when
    /// the store cannot be read, so "empty" means "nobody told us".
    ///
    /// The comparison is [`summo_models::langs_cover`], not a copy of it. This once spelled it
    /// `self.claims.contains(&language)`, which reads every list as literal codes and so answered
    /// "no" to every utterance for the models that publish `langs: ["*"]`.
    /// Shortest utterance whose detected language is worth believing.
    ///
    /// Measured, because this decides whether the bilingual arrangement works at all. Whisper with
    /// no language named, FLEURS clips cut to length, counting how often the answer came back in
    /// the language that was spoken:
    ///
    /// ```text
    ///              0.6 s   1.2 s   2.5 s
    /// whisper-tiny  vi   0%     28%     72%
    /// whisper-base  vi   0%     28%     76%
    /// whisper-tiny  en  12%     56%    100%
    /// whisper-base  en  12%     64%    100%
    /// ```
    ///
    /// Under a second the label is worthless — zero for Vietnamese — and a bigger base model does
    /// not help, because this is a property of the audio and not of the model. Reported as short
    /// phrases coming back as confident nonsense: `alo alo` as `Am I wrong? Am I wrong`, `Excuse
    /// me` as `And over the door`.
    ///
    /// Two seconds is where detection stops being a coin toss. Below it the label is ignored and
    /// the utterance goes to the second model regardless — a wasted decode on a short sentence
    /// costs 20 ms with a specialist at real-time factor 0.019, and the thing it buys is the half
    /// of a bilingual meeting that short phrases live in.
    pub const TRUST_LANGUAGE_ABOVE_S: f64 = 2.0;

    /// Whether this job is worth refining, given how long it is as well as what it claims.
    ///
    /// Separate from [`Self::wants`] because the duration is a fact about the *job* and the claim
    /// is a fact about the pairing, and the tests for the pairing should not have to invent a
    /// length.
    #[must_use]
    pub fn wants_job(&self, language: Option<&str>, seconds: f64) -> bool {
        self.pick(language, seconds).is_some()
    }

    #[must_use]
    pub fn wants(&self, language: Option<&str>) -> bool {
        self.pick(language, Self::TRUST_LANGUAGE_ABOVE_S).is_some()
    }

    /// Which pass should hear this utterance, if any.
    ///
    /// Two questions in order, because they are different questions. *May* a pass have it — the
    /// rules in [`Self::wants`], applied to that pass's own claim. Then, among those that may,
    /// *should* it: a model that names the language beats a model that claims every language,
    /// every time. Whisper claims `*` and so passes the first test for a Japanese sentence;
    /// `sense-voice-small` names `ja` and is four times more accurate on it.
    ///
    /// With one pass this is exactly the old behaviour, which is why the tests below did not have
    /// to change.
    #[must_use]
    pub fn pick(&self, language: Option<&str>, seconds: f64) -> Option<usize> {
        self.candidates(language, seconds).into_iter().next()
    }

    /// Every pass worth asking about this utterance, best first.
    ///
    /// A list rather than a choice, because the label cannot be trusted to make the choice. The
    /// caller runs them in order and keeps the first answer that is not the silence a specialist
    /// gives when fed a language it does not speak — see `too_quiet_to_be_this_language`, which
    /// measured that silence at 15 characters against 146.
    ///
    /// Order: a pass that *names* the reported language first, since when the label is right it is
    /// the cheapest way to be right; then the rest of the declared languages, in the order the
    /// user named them, which is their own answer to "what is this meeting mostly in".
    #[must_use]
    pub fn candidates(&self, language: Option<&str>, seconds: f64) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        let push = |i: usize, out: &mut Vec<usize>| {
            if !out.contains(&i) {
                out.push(i);
            }
        };

        // The label's own pick, when it has one and something claims it.
        if let Some(code) = language {
            for (i, pass) in self.passes.iter().enumerate() {
                if pass.names(code) && self.may(pass, language, seconds) {
                    push(i, &mut out);
                }
            }
        }

        // Then every specialist for a language the user declared. This is the part that was
        // missing: it does not consult the label at all, so a sentence mislabelled `zh` still
        // reaches the Vietnamese model that can hear it.
        for (i, pass) in self.passes.iter().enumerate() {
            if self
                .declared
                .iter()
                .any(|code| pass.names(code) && !self.live_hears_only(code))
            {
                push(i, &mut out);
            }
        }

        // And finally whatever the old rule allows, so a meeting with nothing declared behaves
        // exactly as it did.
        for (i, pass) in self.passes.iter().enumerate() {
            if self.may(pass, language, seconds) {
                push(i, &mut out);
            }
        }
        out
    }

    /// Whether the live model is itself the specialist for this language.
    ///
    /// A specialist listening live has already given its best answer; running a second model over
    /// its own language is the pairing this file spent a release getting backwards.
    fn live_hears_only(&self, code: &str) -> bool {
        !self.live_claims.is_empty()
            && !self.live_claims.iter().any(|l| l == "*")
            && summo_models::langs_cover(&self.live_claims, code)
    }

    /// Whether this pass is allowed the utterance at all.
    fn may(&self, pass: &Pass, language: Option<&str>, seconds: f64) -> bool {
        if seconds < Self::TRUST_LANGUAGE_ABOVE_S {
            // Too short to have been labelled reliably. Ask rather than trusting a coin toss about
            // which language this was. Which pass is asked is still decided by the label, because a
            // guess is better than an arbitrary choice and `too_quiet_to_be_this_language` throws
            // out the answer when the guess was wrong.
            return true;
        }
        if self.second_opinion_only(pass) {
            // The specialist already heard this one better, and an utterance it did not label is
            // in the only language it speaks.
            return language
                .is_some_and(|code| !summo_models::langs_cover(&self.live_claims, code));
        }

        let Some(language) = language else {
            return true;
        };
        if pass.claims.is_empty() {
            return true;
        }
        summo_models::langs_cover(&pass.claims, language)
    }

    /// Whether this pairing is "catch what the live model cannot hear" rather than "hear it again
    /// more carefully".
    ///
    /// A general model under a specialist can only be the first. It knows nothing the specialist
    /// does not about the specialist's own language, and it is measurably worse at it.
    fn second_opinion_only(&self, pass: &Pass) -> bool {
        let general = pass.claims.is_empty() || pass.claims.iter().any(|l| l == "*");
        let specialist = !self.live_claims.is_empty() && !self.live_claims.iter().any(|l| l == "*");
        general && specialist
    }

    /// Start whichever of these jobs are worth starting.
    pub fn dispatch(&self, jobs: Vec<RefineJob>) {
        for job in jobs {
            let candidates = self.candidates(job.language.as_deref(), job.t1 - job.t0);
            if candidates.is_empty() {
                tracing::debug!(
                    seq = job.seq,
                    language = ?job.language,
                    "refine skipped; no second model is for this language"
                );
                // And once, loudly, per language. A pairing that will never run is a decision the
                // user made and got no reply to; this is the line a support question is answered
                // from, and the one `bilingual.mjs` asserts is *absent* for a multilingual model.
                if let Some(language) = job.language.as_deref()
                    && self
                        .told
                        .lock()
                        .is_ok_and(|mut seen| seen.insert(language.to_string()))
                {
                    tracing::info!(
                        language = %language,
                        "the second model claims no such language; those lines keep the first model's text"
                    );
                }
                continue;
            }

            // Only the ones that are free. A busy pass is skipped rather than queued behind
            // itself — but with several candidates, busy no longer means the sentence goes
            // unrescued, which is what made `MAX_IN_FLIGHT` quietly expensive.
            let free: Vec<usize> = candidates
                .into_iter()
                .filter(|&i| self.passes[i].running.load(Ordering::Relaxed) < MAX_IN_FLIGHT)
                .collect();
            if free.is_empty() {
                self.missed.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(
                    seq = job.seq,
                    "refine skipped; every model for this sentence is still on the last one"
                );
                continue;
            }

            // Each candidate's decoder, pulled out before the thread starts so the borrow of
            // `self` ends here.
            let attempts: Vec<Attempt> = free
                .iter()
                .map(|&i| {
                    let pass = &self.passes[i];
                    pass.running.fetch_add(1, Ordering::Relaxed);
                    Attempt {
                        model: pass.id.clone(),
                        decoder: pass.decoder.clone(),
                        // The one language this pass speaks, when it speaks exactly one. A
                        // specialist reports no language per utterance, and without this the
                        // revision kept the *fast* model's guess — which is how a Vietnamese
                        // line rescued by `gipformer` ended up labelled `zh` and then
                        // translated into Vietnamese.
                        speaks: pass.sole(),
                        running: pass.running.clone(),
                    }
                })
                .collect();

            let filter = self.filter.clone();
            let tx = self.tx.clone();

            tokio::task::spawn_blocking(move || {
                // Tried in order, stopping at the first answer that is not silence.
                //
                // `refine` already returns `None` when the model it was given cannot hear this
                // language — that is `too_quiet_to_be_this_language`, and it is a measurement
                // rather than a guess: `gipformer-65m` returns a mean of 146 characters on
                // Vietnamese audio and 15 on English. So the arbitration is the *output*, and the
                // label that has been wrong all along never gets a vote on who is refused.
                let mut winner = None;
                for Attempt {
                    model,
                    decoder,
                    speaks,
                    running,
                } in &attempts
                {
                    if winner.is_none() {
                        let revised = decoder
                            .lock()
                            .map_err(|_| ())
                            .and_then(|mut held| {
                                HybridSession::<Box<dyn Decoder>>::refine(
                                    &job,
                                    held.as_mut(),
                                    &filter,
                                    &job.text,
                                    speaks.as_deref(),
                                )
                                .map_err(|e| {
                                    // A failed refinement costs one line its second opinion. The
                                    // recording continues and the fast model's text stands, which
                                    // is why this is a log rather than an error event: there is
                                    // nothing for the user to do.
                                    tracing::warn!(error = %e, seq = job.seq, "refine pass failed");
                                })
                            })
                            .ok()
                            .flatten();
                        if let Some(event) = revised {
                            winner = Some((model.clone(), event));
                        }
                    }
                    running.fetch_sub(1, Ordering::Relaxed);
                }

                if let Some((model, event)) = winner {
                    // Logged, and at info rather than debug. This is the one observable sign that
                    // the second model ran and disagreed — the transcript changes under the reader
                    // and nothing else says why — so it belongs in the record a support question
                    // would be answered from, and in the one `bilingual.mjs` asserts on.
                    tracing::info!(seq = job.seq, model = %model, "refined an utterance");
                    let _ = tx.send(event);
                }
            });
        }
    }

    /// How many utterances went unrefined because every model for them was busy.
    ///
    /// Read by the status line. A dropped refine used to be `debug` only, on the grounds that the
    /// fast model's text is still correct — which stopped being true when the fast model is the
    /// one producing `今天今天今天`. A sentence nobody rescued is now counted where the count can
    /// be seen.
    #[must_use]
    pub fn missed(&self) -> usize {
        self.missed.load(Ordering::Relaxed)
    }

    /// Revisions that have come back since the last call.
    pub fn collect(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            out.push(event);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_asr::Transcript;

    struct Nothing;
    impl Decoder for Nothing {
        fn decode(&mut self, _pcm: &[f32]) -> summo_core::Result<Transcript> {
            Ok(Transcript::default())
        }
        fn name(&self) -> &str {
            "nothing"
        }
    }

    /// A refiner whose live model is unknown, which is how every test below the new ones reads.
    ///
    /// An empty live claim is "nobody told us", and the second-opinion rule needs a *specialist*
    /// live model to apply — so these keep describing the behaviour they always described.
    fn refiner(claims: &[&str]) -> Refiner {
        paired(claims, &[])
    }

    /// A refiner with both halves named: what the second model claims, and what the live one does.
    fn paired(claims: &[&str], live: &[&str]) -> Refiner {
        Refiner::new(
            claims.join("+"),
            Box::new(Nothing),
            claims.iter().map(|c| (*c).to_string()).collect(),
            live.iter().map(|c| (*c).to_string()).collect(),
            HallucinationFilter::default(),
        )
    }

    /// A trilingual meeting: a multilingual model listening, one specialist per named language.
    fn trilingual() -> Refiner {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.also("sense-voice", Box::new(Nothing), vec!["ja".into()]);
        refiner.also("zipformer-en", Box::new(Nothing), vec!["en".into()]);
        refiner
    }

    /// The feature the `Option` was in the way of.
    ///
    /// Name three languages and only the first got a specialist; the Japanese half of the meeting
    /// was whatever the multilingual model heard. `sense-voice-small` is 8.4 % character error on
    /// Japanese against `whisper-tiny`'s 39.8 %, so this is most of the accuracy of every Japanese
    /// sentence spoken.
    #[test]
    fn every_named_language_reaches_its_own_specialist() {
        let refiner = trilingual();
        assert_eq!(refiner.pick(Some("vi"), 4.0), Some(0));
        assert_eq!(refiner.pick(Some("ja"), 4.0), Some(1));
        assert_eq!(refiner.pick(Some("en"), 4.0), Some(2));
    }

    /// And a language nobody named still goes nowhere, rather than to whichever specialist is first.
    ///
    /// Three specialists that each claim one language claim nothing between them for a fourth. The
    /// live model's own text stands, which is the right answer — running a Vietnamese model on
    /// German returns Vietnamese-shaped noise.
    #[test]
    fn a_language_no_specialist_claims_is_left_to_the_live_model() {
        assert_eq!(trilingual().pick(Some("de"), 4.0), None);
    }

    /// A model that names the language beats one that claims every language.
    ///
    /// Both are eligible under the rules — a general second model may re-hear anything — and only
    /// one of them should get it. Ordering decided this before, so a general model added first
    /// would have taken every utterance in the meeting and the specialists would have idled.
    #[test]
    fn a_specialist_outranks_a_general_model_that_also_wants_it() {
        let mut refiner = paired(&["*"], &["*"]);
        refiner.also("gipformer", Box::new(Nothing), vec!["vi".into()]);
        assert_eq!(refiner.pick(Some("vi"), 4.0), Some(1));
        // And what nobody specialises in still has somewhere to go.
        assert_eq!(refiner.pick(Some("de"), 4.0), Some(0));
    }

    /// Asking for the same model twice loads it once.
    ///
    /// The caller asks for a specialist per named language without working out which names resolve
    /// to the same model, because that is this function's job and not the caller's. `vi` and
    /// `vi-VN` are the obvious pair; any two languages whose best installed model is the same one
    /// are the general case. A duplicate is several hundred megabytes of decoder for nothing.
    #[test]
    fn the_same_model_named_twice_is_loaded_once() {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.also("vi+", Box::new(Nothing), vec!["vi".into()]);
        refiner.also("vi+", Box::new(Nothing), vec!["vi".into()]);
        assert_eq!(refiner.models().len(), 2);
    }

    /// Changing the second model in front of the meeting keeps the others.
    ///
    /// Rebuilding the refiner was the obvious implementation and would drop the specialists the
    /// other named languages depend on — a silent accuracy loss on the half of the meeting the
    /// user was not thinking about when they changed it.
    #[test]
    fn swapping_the_named_second_model_keeps_the_other_specialists() {
        let mut refiner = trilingual();
        refiner.replace_primary("gipformer-65m", Box::new(Nothing), vec!["vi".into()]);
        assert_eq!(
            refiner.models(),
            vec!["gipformer-65m", "sense-voice", "zipformer-en"]
        );
        assert_eq!(refiner.pick(Some("ja"), 4.0), Some(1));
    }

    /// The bug a user found by speaking Vietnamese into the app.
    ///
    /// Gipformer live, Whisper second — which is what `automatic_second` pairs when nobody has
    /// chosen, and Whisper claims `*`. Under the old rule every Vietnamese utterance was handed to
    /// Whisper and its text *replaced* Gipformer's. Whisper-base is 44.2 % CER on Vietnamese
    /// against a model picked for the language; what came back was English fragments and a single
    /// word repeated down the transcript.
    #[test]
    fn a_general_second_model_does_not_re_hear_the_specialists_own_language() {
        let whisper_under_gipformer = paired(&["*"], &["vi"]);
        assert!(
            !whisper_under_gipformer.wants(Some("vi")),
            "the specialist already heard this one, and heard it better"
        );
    }

    /// And the case that did the most damage, because it is the common one.
    ///
    /// A single-language decoder reports no language — there is nothing to report. "An unreported
    /// language is still refined" therefore handed a general model the entire meeting, every
    /// utterance of it, rather than the occasional foreign sentence the pairing exists for.
    #[test]
    fn an_unreported_language_under_a_specialist_is_the_specialists_own() {
        assert!(!paired(&["*"], &["vi"]).wants(None));
    }

    /// What the pairing is actually for, still working.
    ///
    /// `second_opinion` states it: "covers the rest, so a sentence in another language is still
    /// words". An English sentence in a Vietnamese meeting is exactly that sentence.
    #[test]
    fn a_general_second_model_still_catches_what_the_specialist_cannot_hear() {
        let whisper_under_gipformer = paired(&["*"], &["vi"]);
        assert!(whisper_under_gipformer.wants(Some("en")));
        assert!(whisper_under_gipformer.wants(Some("ja")));
    }

    /// The other direction is untouched, and deliberately.
    ///
    /// Whisper live with Gipformer second is the arrangement `e2e/bilingual.mjs` drives with real
    /// audio: the specialist revises the Vietnamese and leaves the English alone. There the second
    /// model's own claim is already the right gate, and narrowing it would delete the feature.
    #[test]
    fn a_specialist_under_a_general_live_model_still_revises_its_own_language() {
        let gipformer_under_whisper = paired(&["vi"], &["*"]);
        assert!(gipformer_under_whisper.wants(Some("vi")));
        assert!(!gipformer_under_whisper.wants(Some("en")));
    }

    /// A short utterance is refined whatever it claims to be, because the claim is a coin toss.
    ///
    /// Measured with Whisper and no language named: under a second, Vietnamese came back labelled
    /// correctly **zero** times out of twenty-five, and a bigger base model scored the same — this
    /// is a property of short audio, not of the model. Reported as `alo alo` transcribed as `Am I
    /// wrong? Am I wrong` on a Vietnamese-and-English call.
    ///
    /// The cost of being wrong the other way is one decode of a short clip. At real-time factor
    /// 0.019 that is about twenty milliseconds.
    #[test]
    fn a_sentence_too_short_to_label_is_refined_anyway() {
        let refiner = paired(&["vi"], &["*"]);

        // Long enough to believe, and in a language the specialist does not claim: skipped.
        assert!(!refiner.wants_job(Some("en"), 4.0));
        // The same claim on a two-word phrase: not believed, so it is asked rather than assumed.
        assert!(refiner.wants_job(Some("en"), 0.6));
        assert!(refiner.wants_job(Some("en"), 1.9));
        // And the boundary is where the measurement put it.
        assert!(!refiner.wants_job(Some("en"), Refiner::TRUST_LANGUAGE_ABOVE_S));
    }

    /// Two specialists is not a second opinion either way, so the old rule stands.
    #[test]
    fn two_specialists_are_gated_by_the_second_models_own_claim() {
        let english_under_vietnamese = paired(&["en"], &["vi"]);
        assert!(english_under_vietnamese.wants(Some("en")));
        assert!(!english_under_vietnamese.wants(Some("vi")));
    }

    /// A regional tag asks for its base language, here as everywhere else.
    #[test]
    fn a_regional_tag_counts_as_the_specialists_language() {
        assert!(!paired(&["*"], &["en"]).wants(Some("en-US")));
        assert!(paired(&["*"], &["en"]).wants(Some("vi")));
    }

    /// The whole point of the feature: an English sentence in a Vietnamese meeting is left alone by
    /// a Vietnamese-only model. Running it would replace real English with Vietnamese-shaped noise
    /// — a worse line than the one it overwrote, which is the failure that makes "refine
    /// everything" the wrong reading of this setting.
    #[test]
    fn a_model_is_not_run_on_a_language_it_does_not_claim() {
        let vietnamese_only = refiner(&["vi"]);
        assert!(vietnamese_only.wants(Some("vi")));
        assert!(!vietnamese_only.wants(Some("en")));
    }

    /// Case is the runtime's business, not the user's. Whisper answers `<|EN|>` on some builds and
    /// a manifest says `en`; comparing them raw silently refuses every utterance.
    #[test]
    fn the_comparison_does_not_care_about_case() {
        assert!(refiner(&["VI"]).wants(Some("vi")));
        assert!(refiner(&["vi"]).wants(Some("VI")));
    }

    /// A decoder that was *told* its language reports none, and that is the single-language setup
    /// somebody configured on purpose. Skipping it would turn the setting off for precisely the
    /// people who meant it.
    #[test]
    fn an_unreported_language_is_still_refined() {
        assert!(refiner(&["vi"]).wants(None));
    }

    /// A registry entry with no languages is an incomplete manifest, not a model that is good for
    /// nothing. Reading it as a claim on nothing would make the feature silently do nothing.
    #[test]
    fn a_model_that_claims_nothing_is_treated_as_claiming_everything() {
        assert!(refiner(&[]).wants(Some("en")));
        assert!(refiner(&[]).wants(Some("vi")));
    }

    /// The multilingual spelling, which is the one this file got wrong.
    ///
    /// `whisper-base` and `whisper-tiny` both publish `langs: ["*"]`, and pairing one of them as
    /// the second opinion over a specialised live model is the most ordinary way to use this
    /// feature. A star matches no language code, so `contains` said no to every utterance and the
    /// refine pass ran on nothing at all — reported only at `debug`, so the setting looked applied,
    /// `/status` named the model, and the transcript was never revised.
    #[test]
    fn a_multilingual_model_claims_every_language() {
        let whisper = refiner(&["*"]);
        assert!(whisper.wants(Some("en")));
        assert!(whisper.wants(Some("vi")));
        assert!(whisper.wants(Some("ja")));
        assert!(whisper.wants(None));
    }

    /// A star beside real codes is still a star. Nothing publishes this today, and reading the list
    /// as "only the codes spelled out" would be a quieter version of the same bug.
    #[test]
    fn a_star_anywhere_in_the_list_covers_everything() {
        assert!(refiner(&["vi", "*"]).wants(Some("de")));
    }

    /// A region tag is a spelling of the language, not a different one. `en-US` from a runtime must
    /// not miss a manifest that says `en`.
    #[test]
    fn a_region_tag_matches_the_language_it_is_a_region_of() {
        assert!(refiner(&["en"]).wants(Some("en-US")));
        assert!(!refiner(&["en"]).wants(Some("de-DE")));
    }

    /// The bug a real meeting produced, in one assertion.
    ///
    /// Vietnamese and English declared, `whisper-tiny` listening, `gipformer` installed for
    /// Vietnamese. Whisper labelled Vietnamese speech `zh`, `he` and `ja` — no specialist claims
    /// those — so the rescue path refused every one of them and the transcript kept
    /// `大家看今天今天今天…` and `- Now, הנה, הנה,`.
    ///
    /// The declaration is a fact the user gave us. The label is a guess that this meeting proved
    /// wrong well above the two seconds it was trusted from.
    #[test]
    fn a_declared_language_reaches_its_specialist_whatever_the_label_says() {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.declared(&["vi".into(), "en".into()]);

        for invented in ["zh", "he", "ja", "ru"] {
            assert_eq!(
                refiner.candidates(Some(invented), 4.0),
                vec![0],
                "a line labelled {invented} never reached the Vietnamese model"
            );
        }
    }

    /// And the label still goes first when it names something.
    ///
    /// Being right cheaply matters: when the guess is correct, trying that model first saves every
    /// other decode. It only stops being allowed to *refuse*.
    #[test]
    fn the_label_still_orders_the_attempts() {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.also("zipformer-en", Box::new(Nothing), vec!["en".into()]);
        refiner.declared(&["vi".into(), "en".into()]);

        assert_eq!(refiner.candidates(Some("en"), 4.0).first(), Some(&1));
        assert_eq!(refiner.candidates(Some("vi"), 4.0).first(), Some(&0));
        // And both are tried, because either could be the one that hears it.
        assert_eq!(refiner.candidates(Some("en"), 4.0).len(), 2);
    }

    /// A specialist listening live is not second-guessed on its own language.
    ///
    /// The pairing this file spent a release getting backwards: Gipformer live with Whisper
    /// second, and Whisper re-decoding every Vietnamese line at 44.2 % character error. Declaring
    /// Vietnamese must not reopen that door.
    #[test]
    fn declaring_a_language_the_live_model_specialises_in_changes_nothing() {
        let mut refiner = paired(&["*"], &["vi"]);
        refiner.declared(&["vi".into()]);
        assert!(refiner.candidates(Some("vi"), 4.0).is_empty());
        // The sentence it *cannot* hear still goes to the general model. That is the pairing.
        assert_eq!(refiner.candidates(Some("en"), 4.0), vec![0]);
    }

    /// Declaring nothing leaves the old behaviour exactly as it was.
    #[test]
    fn with_nothing_declared_the_label_still_decides() {
        let refiner = paired(&["vi"], &["*"]);
        assert_eq!(refiner.candidates(Some("vi"), 4.0), vec![0]);
        assert!(refiner.candidates(Some("en"), 4.0).is_empty());
    }

    /// A declared language nothing specialises in is still nobody's job.
    #[test]
    fn declaring_a_language_no_model_speaks_summons_nothing() {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.declared(&["vi".into(), "ko".into()]);
        // `ko` has no specialist; the Vietnamese one is offered because Vietnamese is declared,
        // and `too_quiet_to_be_this_language` is what will refuse its answer on Korean audio.
        assert_eq!(refiner.candidates(Some("ko"), 4.0), vec![0]);
    }

    /// The other half of the user's question, and the half a label filter cannot reach.
    ///
    /// *"Khai vi+en thì chạy whisper vi+en thôi, chạy zh he ja làm gì? Lọc ra ok nhưng mà không
    /// đúng."* Correct: whisper's `language` is the decoder's prompt, so a sentence it guesses is
    /// Chinese comes back **in Chinese characters**, and correcting the label afterwards changes
    /// nothing about the text. A declared language nothing specialises in gets the general model
    /// pinned to it instead, and then it cannot wander.
    #[test]
    fn a_pinned_general_model_hears_only_what_it_was_pinned_to() {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.pinned_to("whisper-tiny", "en", Box::new(Nothing), vec!["*".into()]);
        refiner.declared(&["vi".into(), "en".into()]);

        // English reaches the pinned Whisper, Vietnamese reaches the specialist.
        assert_eq!(refiner.candidates(Some("en"), 4.0).first(), Some(&1));
        assert_eq!(refiner.candidates(Some("vi"), 4.0).first(), Some(&0));
        // And a Vietnamese line the fast model called Chinese still reaches both, in that order,
        // with the output deciding — which is the arrangement this file already had.
        assert_eq!(refiner.candidates(Some("zh"), 4.0), vec![0, 1]);
    }

    /// The same model pinned to two languages is two passes, not one.
    ///
    /// Deduplication used to key on the model id. `whisper-tiny[en]` and `whisper-tiny[ja]` are
    /// one id and two different questions, and keying on the id would have kept the first and
    /// silently dropped the second — the shape of bug this file has now fixed twice.
    #[test]
    fn one_model_pinned_to_two_languages_is_two_passes() {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.pinned_to("whisper-tiny", "en", Box::new(Nothing), vec!["*".into()]);
        refiner.pinned_to("whisper-tiny", "ja", Box::new(Nothing), vec!["*".into()]);
        refiner.declared(&["vi".into(), "en".into(), "ja".into()]);

        assert_eq!(
            refiner.models(),
            vec!["vi", "whisper-tiny[en]", "whisper-tiny[ja]"]
        );
        assert_eq!(refiner.candidates(Some("ja"), 4.0).first(), Some(&2));
    }

    /// Pinning the same model to the same language twice loads it once.
    #[test]
    fn pinning_the_same_language_twice_is_one_pass() {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.pinned_to("whisper-tiny", "en", Box::new(Nothing), vec!["*".into()]);
        refiner.pinned_to("whisper-tiny", "en", Box::new(Nothing), vec!["*".into()]);
        assert_eq!(refiner.models().len(), 2);
    }

    /// A language that already has a specialist is not pinned as well.
    ///
    /// `hears` is what the caller asks before spending a few hundred megabytes on a decoder that
    /// would lose the comparison anyway.
    #[test]
    fn a_language_with_a_specialist_needs_no_pinned_model() {
        let mut refiner = paired(&["vi"], &["*"]);
        refiner.also("sense-voice", Box::new(Nothing), vec!["ja".into()]);

        assert!(refiner.hears("vi"));
        assert!(refiner.hears("ja"));
        assert!(!refiner.hears("en"), "nothing here was built for English");
        // A regional spelling is the same language.
        assert!(refiner.hears("vi-VN"));
    }
}
