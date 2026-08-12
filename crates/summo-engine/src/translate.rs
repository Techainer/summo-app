//! Translating a meeting, in batches, without losing the lines the model drops.
//!
//! `summo_llm::prompt::translate` has been written since the first week and never called. It takes
//! a run of numbered lines because a sentence translated alone loses pronouns and register — and
//! Vietnamese, where the pronoun encodes the relationship between the speakers, punishes that
//! harder than most languages.
//!
//! The thing this module exists to get right is *alignment*. Models drop numbered lines, merge two
//! into one, and occasionally answer with prose. `parse_translation` already returns `None` for a
//! line that did not come back; the job here is to make sure a `None` leaves that one utterance
//! untranslated instead of shifting every later line by one. A transcript where every translation
//! is attached to the previous sentence is worse than no translation at all, because it looks
//! right.

use futures::stream::{self, StreamExt};
use summo_core::{Error, MeetingId, Result, paths::Paths, settings::Settings};
use summo_llm::{LlmClient, prompt};
use summo_vault::{MeetingDoc, translation::Translation};

/// Lines per request.
///
/// Enough that the model sees context around each sentence; small enough that one bad response
/// costs a paragraph rather than the meeting, and that the numbering stays inside what a model
/// tracks reliably. Above about forty, dropped lines become common.
pub const BATCH: usize = 25;

/// Lines in flight at once against a dedicated translation model.
///
/// One at a time wastes most of the machine: a 1B model on a CPU spends about 900 ms a line and
/// leaves the other cores idle, which is fifteen minutes for an hour-long meeting. Four is what
/// `llama-server` runs in parallel out of the box, and going past a server's slot count does not
/// make it faster — the requests queue, and the only thing that grows is how long a cancel takes to
/// take effect.
pub const MT_CONCURRENCY: usize = 4;

/// Which shape of request a translation goes out as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Numbered batches to a general instruction-following model.
    Chat,
    /// One line at a time to a dedicated translation model, in the template it was trained on.
    Mt,
}

/// The model translation goes to, and how to talk to it.
///
/// This exists because "which model" and "which prompt" are the same decision and were previously
/// two. A dedicated translation model given the numbered-batch prompt does not translate — measured
/// against MiLMMT-46-1B, it answered a three-line batch by inventing a fourth line in the source
/// language — so a configuration that could pair the wrong two would be a configuration that
/// silently produces nonsense.
pub struct Translator {
    client: LlmClient,
    style: Style,
    /// The language being spoken, when Summo knows it. Only [`Style::Mt`] uses it, and only as a
    /// hint: see [`summo_llm::prompt::mt`] for why a wrong answer here is cheap.
    source: Option<String>,
}

impl Translator {
    /// A general model, prompted with numbered batches.
    pub fn chat(provider: summo_llm::Provider) -> Result<Self> {
        Ok(Self {
            client: LlmClient::new(provider)?,
            style: Style::Chat,
            source: None,
        })
    }

    /// A dedicated translation model, prompted one line at a time.
    ///
    /// Takes a provider rather than a client so it can pin the temperature, which is not a tuning
    /// preference here. At the shared default of 0.2 a 1B translation model wanders: translating a
    /// three-line Vietnamese meeting into Japanese, the third line came back in **Thai** — fluent,
    /// correct Thai, in a file labelled `ja`. Nothing downstream can catch that, because a wrong
    /// language is not a malformed response.
    ///
    /// Sampling buys nothing anyway. There is one right translation of a sentence and no reason to
    /// want a different one on the next run, which is also what makes a re-translation reproducible.
    pub fn mt(mut provider: summo_llm::Provider, source: Option<String>) -> Result<Self> {
        provider.temperature = 0.0;
        Ok(Self {
            client: LlmClient::new(provider)?,
            style: Style::Mt,
            source,
        })
    }

    /// Whichever the settings ask for: the dedicated translator when one is configured, otherwise
    /// the general model.
    ///
    /// `source` comes from `models.language`, the language the speech model was told to expect. It
    /// is `None` when recognition was left on auto, which is a normal state and not an error.
    pub fn from_settings(paths: &Paths, settings: &Settings) -> Result<Self> {
        let catalogue = summo_llm::provider::catalogue(&paths.providers());
        match &settings.llm.translator {
            Some(mt) => {
                let provider = summo_llm::Provider::resolve_in(
                    &catalogue,
                    &mt.provider,
                    mt.model.as_deref(),
                    None,
                )?;
                Self::mt(provider, settings.models.language.clone())
            }
            None => {
                let provider = summo_llm::Provider::resolve_in(
                    &catalogue,
                    &settings.llm.provider,
                    settings.llm.model.as_deref(),
                    None,
                )?;
                Self::chat(provider)
            }
        }
    }

    #[must_use]
    pub fn style(&self) -> Style {
        self.style
    }

    /// The sampling temperature this translator will use, for the test that pins it at zero.
    #[must_use]
    pub fn temperature(&self) -> f32 {
        self.client.provider().temperature
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.client.provider().model
    }

    /// Translate one run of lines, returning one slot per input line and the requests it cost.
    ///
    /// A `None` slot means the model did not answer for that line. It is never filled in from a
    /// neighbour: a transcript where every translation is attached to the previous sentence is
    /// worse than no translation at all, because it looks right.
    pub async fn run(
        &self,
        lines: &[&str],
        lang: &str,
        glossary: &prompt::Glossary,
    ) -> Result<(Vec<Option<String>>, usize)> {
        match self.style {
            Style::Chat => {
                let messages = prompt::translate(lines, lang, glossary);
                let response = self.client.complete(&messages).await?;
                Ok((prompt::parse_translation(&response, lines.len()), 1))
            }
            Style::Mt => {
                // Per line, several at a time. `buffered` keeps the results in input order however
                // the responses come back, which is what makes the alignment above safe.
                let source = self.source.as_deref();
                // Futures built up front rather than through `stream::iter(..).map(..)`: the
                // closure form needs to be callable for any lifetime, and a borrowed `&&str` from
                // the slice is not.
                let calls: Vec<_> = lines
                    .iter()
                    .copied()
                    .map(|line: &str| async move {
                        let messages = prompt::mt(line, source, lang);
                        let response = self.client.complete(&messages).await?;
                        Ok::<_, Error>(
                            prompt::parse_mt(&response).filter(|text| {
                                // A small translation model sometimes answers in a language nobody
                                // asked for. Measured: MiLMMT-46-1B, given a Vietnamese line and
                                // asked for Japanese, returned fluent **Thai** — at temperature
                                // zero, reproducibly. Writing it would put a language the reader
                                // cannot read into a file labelled with one they can.
                                //
                                // Dropped rather than retried: a `None` here leaves the original
                                // Vietnamese in place, which is a line the user can at least read
                                // and check, and the outcome already reports it as missing.
                                let ok = summo_llm::lang::plausible(text, lang);
                                if !ok {
                                    tracing::warn!(
                                        %lang,
                                        "a translated line came back in another language; keeping the original"
                                    );
                                }
                                ok
                            }),
                        )
                    })
                    .collect();
                let results: Vec<Result<Option<String>>> =
                    stream::iter(calls).buffered(MT_CONCURRENCY).collect().await;

                let mut out = Vec::with_capacity(lines.len());
                for result in results {
                    out.push(result?);
                }
                let requests = out.len();
                Ok((out, requests))
            }
        }
    }

    /// How many lines to send per call to [`Self::run`].
    fn batch(&self) -> usize {
        match self.style {
            Style::Chat => BATCH,
            // Each line is its own request anyway; the chunk only decides how often progress is
            // written, and a chunk of `MT_CONCURRENCY` would leave the pipeline empty between them.
            Style::Mt => MT_CONCURRENCY * 8,
        }
    }
}

/// What a translation run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub lang: String,
    /// Lines the model returned.
    pub translated: usize,
    /// Lines it did not, which keep their original text.
    pub missing: usize,
    /// Requests made, for the cost line in settings.
    pub requests: usize,
}

impl Outcome {
    #[must_use]
    pub fn complete(&self) -> bool {
        self.missing == 0
    }
}

/// Translate every utterance in a meeting into `lang`, writing a translation file beside it.
///
/// Already-translated lines are skipped, so running this twice after adding ten minutes of audio
/// costs ten minutes of translation rather than the whole meeting. `force` re-does everything, for
/// when the glossary changed and the old output is wrong rather than missing.
pub async fn translate(
    paths: &Paths,
    translator: &Translator,
    meeting: &MeetingId,
    doc: &MeetingDoc,
    lang: &str,
    glossary: &prompt::Glossary,
    force: bool,
) -> Result<Outcome> {
    let lang = lang.trim();
    if lang.is_empty() {
        return Err(Error::msg("translate.no_target", "dịch sang ngôn ngữ nào?"));
    }

    let mut existing = if force {
        None
    } else {
        summo_vault::translation::load(paths, meeting, lang)?
    };

    let mut out = existing.take().unwrap_or_else(|| Translation::new(lang));
    out.lang = summo_vault::translation::sanitize_lang(lang);
    out.model = Some(translator.model().to_string());

    // Only the utterances that still need doing, in transcript order.
    let todo: Vec<_> = doc
        .transcript
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .filter(|s| force || out.get(s.seq).is_none())
        .collect();

    let mut translated = 0usize;
    let mut missing = 0usize;
    let mut requests = 0usize;

    for batch in todo.chunks(translator.batch()) {
        let lines: Vec<&str> = batch.iter().map(|s| s.text.trim()).collect();
        let (parsed, spent) = translator.run(&lines, lang, glossary).await?;
        requests += spent;

        for (segment, text) in batch.iter().zip(parsed) {
            match text {
                Some(text) => {
                    out.set(segment.seq, segment.t0, text);
                    translated += 1;
                }
                // Deliberately not written: an absent line must stay absent so `get` falls back to
                // the original, rather than being filled with the neighbouring sentence.
                None => missing += 1,
            }
        }
    }

    summo_vault::translation::save(paths, meeting, &out)?;

    Ok(Outcome {
        lang: out.lang,
        translated,
        missing,
        requests,
    })
}

/// A copy of the meeting with every translated line swapped in.
///
/// Built for export: `summo_vault::export` renders a `MeetingDoc`, so subtitles in another language
/// are the same code path as subtitles in the original one. Lines with no translation keep their
/// original text — a subtitle file with holes in it is not a subtitle file.
#[must_use]
pub fn applied(doc: &MeetingDoc, translation: &Translation) -> MeetingDoc {
    let mut out = doc.clone();
    for segment in &mut out.transcript {
        if let Some(text) = translation.get(segment.seq) {
            segment.text = text.to_string();
            // The word timings belong to the original audio; keeping them would put the wrong
            // words under the playhead in every karaoke-style highlight.
            segment.words.clear();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::segment::{Lane, Segment};
    use summo_vault::meeting::Frontmatter;

    fn doc_with(texts: &[&str]) -> MeetingDoc {
        let mut doc = MeetingDoc::new(Frontmatter::new(MeetingId::new(), "2026-08-10"), "Họp");
        for (i, text) in texts.iter().enumerate() {
            let seq = i as u64 + 1;
            let t = i as f64 * 10.0;
            doc.transcript
                .push(Segment::new(seq, Lane::System, *text, t, t + 5.0));
        }
        doc
    }

    #[test]
    fn applying_a_translation_swaps_the_text_and_keeps_the_timing() {
        let doc = doc_with(&["xin chào", "cảm ơn"]);
        let mut t = Translation::new("en");
        t.set(1, 0.0, "hello");

        let applied = applied(&doc, &t);
        assert_eq!(applied.transcript[0].text, "hello");
        assert_eq!(applied.transcript[0].t0, doc.transcript[0].t0);
        assert_eq!(applied.transcript[0].t1, doc.transcript[0].t1);
    }

    /// A subtitle file with gaps where the model dropped a line is unusable. The original is a
    /// worse subtitle than a translation and a much better one than nothing.
    #[test]
    fn an_untranslated_line_keeps_its_original_text() {
        let doc = doc_with(&["xin chào", "cảm ơn"]);
        let mut t = Translation::new("en");
        t.set(1, 0.0, "hello");
        assert_eq!(applied(&doc, &t).transcript[1].text, "cảm ơn");
    }

    /// Word timings were measured against Vietnamese audio. Carrying them onto English text would
    /// highlight the wrong word under the playhead for the whole line.
    #[test]
    fn word_timings_are_dropped_on_a_line_that_was_translated() {
        let mut doc = doc_with(&["xin chào"]);
        doc.transcript[0].words = vec![summo_core::segment::Word {
            text: "xin".into(),
            t0: 0.0,
            t1: 0.5,
            conf: None,
        }];
        let mut t = Translation::new("en");
        t.set(1, 0.0, "hello");

        assert!(applied(&doc, &t).transcript[0].words.is_empty());
    }

    #[test]
    fn word_timings_survive_on_a_line_that_was_not() {
        let mut doc = doc_with(&["xin chào"]);
        doc.transcript[0].words = vec![summo_core::segment::Word {
            text: "xin".into(),
            t0: 0.0,
            t1: 0.5,
            conf: None,
        }];
        let applied = applied(&doc, &Translation::new("en"));
        assert_eq!(applied.transcript[0].words.len(), 1);
    }

    #[test]
    fn a_run_with_nothing_missing_reports_itself_complete() {
        let outcome = Outcome {
            lang: "en".into(),
            translated: 10,
            missing: 0,
            requests: 1,
        };
        assert!(outcome.complete());
    }

    #[tokio::test]
    async fn an_empty_target_language_is_refused_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        let provider = summo_llm::Provider::custom("x", "https://127.0.0.1:1", "m");
        let doc = doc_with(&["xin chào"]);

        let err = translate(
            &paths,
            &Translator::chat(provider).unwrap(),
            &MeetingId::new(),
            &doc,
            "  ",
            &prompt::Glossary::default(),
            false,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("ngôn ngữ"), "{err}");
    }

    /// A meeting with nothing to say must not spend a request finding that out — and must still
    /// leave a file, so the UI can show "translated, empty" rather than "never translated".
    #[tokio::test]
    async fn a_transcript_of_only_blank_lines_costs_no_request() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        // Unreachable on purpose: reaching the model would fail the test.
        let provider = summo_llm::Provider::custom("x", "https://127.0.0.1:1", "m");
        let id = MeetingId::new();
        let doc = doc_with(&["   ", ""]);

        let outcome = translate(
            &paths,
            &Translator::chat(provider).unwrap(),
            &id,
            &doc,
            "en",
            &prompt::Glossary::default(),
            false,
        )
        .await
        .expect("no request needed");

        assert_eq!(outcome.requests, 0);
        assert!(
            summo_vault::translation::load(&paths, &id, "en")
                .unwrap()
                .is_some()
        );
    }
}
