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

use summo_core::{Error, MeetingId, Result, paths::Paths};
use summo_llm::{LlmClient, prompt};
use summo_vault::{MeetingDoc, translation::Translation};

/// Lines per request.
///
/// Enough that the model sees context around each sentence; small enough that one bad response
/// costs a paragraph rather than the meeting, and that the numbering stays inside what a model
/// tracks reliably. Above about forty, dropped lines become common.
pub const BATCH: usize = 25;

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
    client: &LlmClient,
    meeting: &MeetingId,
    doc: &MeetingDoc,
    lang: &str,
    glossary: &prompt::Glossary,
    force: bool,
) -> Result<Outcome> {
    let lang = lang.trim();
    if lang.is_empty() {
        return Err(Error::Other("dịch sang ngôn ngữ nào?".into()));
    }

    let mut existing = if force {
        None
    } else {
        summo_vault::translation::load(paths, meeting, lang)?
    };

    let mut out = existing.take().unwrap_or_else(|| Translation::new(lang));
    out.lang = summo_vault::translation::sanitize_lang(lang);
    out.model = Some(client.provider().model.clone());

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

    for batch in todo.chunks(BATCH) {
        let lines: Vec<&str> = batch.iter().map(|s| s.text.trim()).collect();
        let messages = prompt::translate(&lines, lang, glossary);
        let response = client.complete(&messages).await?;
        requests += 1;

        let parsed = prompt::parse_translation(&response, lines.len());
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
        let client =
            summo_llm::LlmClient::new(summo_llm::Provider::custom("x", "https://127.0.0.1:1", "m"))
                .unwrap();
        let doc = doc_with(&["xin chào"]);

        let err = translate(
            &paths,
            &client,
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
        let client =
            summo_llm::LlmClient::new(summo_llm::Provider::custom("x", "https://127.0.0.1:1", "m"))
                .unwrap();
        let id = MeetingId::new();
        let doc = doc_with(&["   ", ""]);

        let outcome = translate(
            &paths,
            &client,
            &id,
            &doc,
            "en",
            &prompt::Glossary::default(),
            false,
        )
        .await
        .expect("no request needed");

        assert_eq!(outcome.requests, 0);
        assert!(summo_vault::translation::load(&paths, &id, "en").unwrap().is_some());
    }
}
