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

/// Refine passes allowed to be running at once.
///
/// One. The decoder holds mutable inference state and cannot be shared, so a second concurrent job
/// would only wait on the mutex — with the difference that it would hold a blocking thread while it
/// waited. Jobs past this are dropped by the queue in `stages.rs`, which is the right answer: a
/// refine model that cannot keep up is not improving the transcript anybody is reading.
const MAX_IN_FLIGHT: usize = 1;

/// The slower model, and what it is worth running on.
pub struct Refiner {
    /// Behind a mutex because the work happens on a pool thread and a decoder is not `Sync`.
    decoder: Arc<Mutex<Box<dyn Decoder>>>,
    /// The languages the refine model's manifest claims. Empty means "no claim on record".
    claims: Vec<String>,
    filter: HallucinationFilter,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
    running: Arc<AtomicUsize>,
    /// Languages already reported as outside this model's claim.
    ///
    /// So the notice below is said once rather than once per sentence. The per-utterance skip stays
    /// at `debug` for the reason given at its call site — a stream of them during a fast
    /// conversation is not news — but *that the pairing does nothing at all* is, and it was the one
    /// thing nobody could see: a refine model that never runs looks exactly like one that runs and
    /// agrees. `/status` names it either way.
    told: Mutex<BTreeSet<String>>,
}

impl Refiner {
    #[must_use]
    pub fn new(
        decoder: Box<dyn Decoder>,
        claims: Vec<String>,
        filter: HallucinationFilter,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            decoder: Arc::new(Mutex::new(decoder)),
            claims: claims.into_iter().map(|l| l.to_lowercase()).collect(),
            filter,
            tx,
            rx,
            running: Arc::new(AtomicUsize::new(0)),
            told: Mutex::new(BTreeSet::new()),
        }
    }

    /// Whether this model is the right one for what was just heard.
    ///
    /// See the module note: an unclaimed language is skipped, an unreported one is not. A manifest
    /// with no languages at all is treated as a claim on everything, because that is a gap in the
    /// registry entry rather than a statement that the model is good for nothing — and here the gap
    /// is usually not even in the file: [`Refiner::new`]'s callers read the langs through
    /// `store.installed(…)` and fall back to an empty list when that read fails, so "empty" means
    /// "nobody told us", which is not grounds for refusing to run.
    ///
    /// The comparison itself is [`summo_models::langs_cover`], not a copy of it. This method used
    /// to spell it `self.claims.contains(&language)`, which reads every list as literal codes and
    /// so answered "no" to every utterance for the two models that publish `langs: ["*"]` — the
    /// Whispers, which are precisely the models worth pairing as a second opinion over a
    /// specialised live one.
    #[must_use]
    pub fn wants(&self, language: Option<&str>) -> bool {
        let Some(language) = language else {
            return true;
        };
        if self.claims.is_empty() {
            return true;
        }
        summo_models::langs_cover(&self.claims, language)
    }

    /// Start whichever of these jobs are worth starting.
    pub fn dispatch(&self, jobs: Vec<RefineJob>) {
        for job in jobs {
            if !self.wants(job.language.as_deref()) {
                tracing::debug!(
                    seq = job.seq,
                    language = ?job.language,
                    "refine skipped; the second model does not claim this language"
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
            if self.running.load(Ordering::Relaxed) >= MAX_IN_FLIGHT {
                // Reported nowhere on purpose. The line the user is reading is correct as far as
                // the fast model is concerned; that a second opinion was skipped is not news, and
                // a notice per dropped job during a fast conversation would be a stream of them.
                tracing::debug!(
                    seq = job.seq,
                    "refine skipped; the model is still on the last one"
                );
                continue;
            }

            let decoder = self.decoder.clone();
            let filter = self.filter.clone();
            let tx = self.tx.clone();
            let running = self.running.clone();

            running.fetch_add(1, Ordering::Relaxed);
            tokio::task::spawn_blocking(move || {
                let revised = decoder
                    .lock()
                    .map_err(|_| ())
                    .and_then(|mut held| {
                        HybridSession::<Box<dyn Decoder>>::refine(
                            &job,
                            held.as_mut(),
                            &filter,
                            &job.text,
                        )
                        .map_err(|e| {
                            // A failed refinement costs one line its second opinion. The recording
                            // continues and the fast model's text stands, which is why this is a
                            // log rather than an error event: there is nothing for the user to do.
                            tracing::warn!(error = %e, seq = job.seq, "refine pass failed");
                        })
                    })
                    .ok()
                    .flatten();
                running.fetch_sub(1, Ordering::Relaxed);
                if let Some(event) = revised {
                    // Logged, and at info rather than debug. This is the one observable sign that
                    // the second model ran and disagreed — the transcript changes under the reader
                    // and nothing else says why — so it belongs in the record a support question
                    // would be answered from, and in the one `bilingual.mjs` asserts on.
                    tracing::info!(seq = job.seq, "refined an utterance");
                    let _ = tx.send(event);
                }
            });
        }
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

    fn refiner(claims: &[&str]) -> Refiner {
        Refiner::new(
            Box::new(Nothing),
            claims.iter().map(|c| (*c).to_string()).collect(),
            HallucinationFilter::default(),
        )
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
}
