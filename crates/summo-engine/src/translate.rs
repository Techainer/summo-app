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

/// Which shape of request a translation goes out as, and where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Numbered batches to a general instruction-following model, over HTTP.
    Chat,
    /// One line at a time to a dedicated translation model, over HTTP, in the template it was
    /// trained on. For somebody already running Ollama or llama.cpp.
    Mt,
    /// The same template, to a model loaded in this process. Nothing else to install and nothing
    /// else to keep running, which is what makes translation free in practice rather than only in
    /// principle.
    Local,
}

/// The model translation goes to, and how to talk to it.
///
/// This exists because "which model" and "which prompt" are the same decision and were previously
/// two. A dedicated translation model given the numbered-batch prompt does not translate — measured
/// against MiLMMT-46-1B, it answered a three-line batch by inventing a fourth line in the source
/// language — so a configuration that could pair the wrong two would be a configuration that
/// silently produces nonsense.
pub struct Translator {
    backend: Backend,
    style: Style,
    /// The language being spoken, when Summo knows it. Only [`Style::Mt`] uses it, and only as a
    /// hint: see [`summo_llm::prompt::mt`] for why a wrong answer here is cheap.
    source: Option<String>,
}

/// Where the request actually goes.
///
/// An enum rather than two `Option`s, so "an HTTP translator with no client" is not a state that
/// can be constructed. The `Local` arm only exists when the feature that can build it does.
enum Backend {
    Http(LlmClient),
    #[cfg(feature = "local-mt")]
    Local(std::sync::Arc<LocalModel>),
}

/// A translation model in this process, whichever runtime it needs.
///
/// Two runtimes because there are two shapes of translation model and neither runs the other's:
/// llama.cpp serves decoder-only GGUF, ONNX Runtime serves the encoder–decoder M2M100 family. Which
/// one a model needs is a fact about the model, so it is read from the manifest rather than from a
/// setting somebody could get wrong.
#[cfg(feature = "local-mt")]
pub enum LocalModel {
    // Boxed: an ONNX session plus two 128 000-entry vocabularies is an order of magnitude larger
    // than a llama.cpp handle, and an unboxed enum is as big as its biggest variant everywhere it
    // is moved.
    Gguf(Box<summo_mt::Local>),
    Onnx(Box<summo_mt::Seq2Seq>),
}

#[cfg(feature = "local-mt")]
impl LocalModel {
    fn name(&self) -> &str {
        match self {
            Self::Gguf(model) => model.name(),
            Self::Onnx(model) => model.name(),
        }
    }

    /// Translate one line.
    ///
    /// The two runtimes want different things and the difference is not cosmetic: a decoder-only
    /// model continues a prompt, so it gets the template it was trained on and its reply is cut at
    /// the first newline. A seq2seq model is *given* a target language as a token and generates
    /// only the translation, so a prompt would be translated along with the sentence.
    fn translate(&self, line: &str, source: Option<&str>, lang: &str) -> Result<Option<String>> {
        match self {
            Self::Gguf(model) => {
                let prompt = prompt::mt_text(line, source, lang);
                Ok(prompt::parse_mt(&model.complete(&prompt)?))
            }
            Self::Onnx(model) => {
                let text = model.translate(line, lang)?;
                Ok((!text.trim().is_empty()).then(|| text.trim().to_string()))
            }
        }
    }
}

impl Translator {
    /// A general model, prompted with numbered batches.
    pub fn chat(provider: summo_llm::Provider) -> Result<Self> {
        Ok(Self {
            backend: Backend::Http(LlmClient::new(provider)?),
            style: Style::Chat,
            source: None,
        })
    }

    /// A translation model loaded in this process.
    ///
    /// Takes an `Arc` because the model is hundreds of megabytes of weights and the daemon builds a
    /// translator per request. Loading it per request would spend a second and a gigabyte to
    /// translate one line.
    #[cfg(feature = "local-mt")]
    #[must_use]
    pub fn local(model: std::sync::Arc<LocalModel>, source: Option<String>) -> Self {
        Self {
            backend: Backend::Local(model),
            style: Style::Local,
            source,
        }
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
            backend: Backend::Http(LlmClient::new(provider)?),
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
            Some(mt) if mt.is_local() => local_translator(paths, settings, mt),
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
    ///
    /// `0.0` for the in-process model too, and not by configuration: `summo_mt` decodes greedily
    /// and has no temperature to set.
    #[must_use]
    pub fn temperature(&self) -> f32 {
        match &self.backend {
            Backend::Http(client) => client.provider().temperature,
            #[cfg(feature = "local-mt")]
            Backend::Local(_) => 0.0,
        }
    }

    /// What goes in the translation file's header, so a reader can tell which model wrote it.
    #[must_use]
    pub fn model(&self) -> &str {
        match &self.backend {
            Backend::Http(client) => &client.provider().model,
            #[cfg(feature = "local-mt")]
            Backend::Local(local) => local.name(),
        }
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
        // Allowed, not fixed: with `local-mt` this match has two arms and is exactly right; without
        // it `Backend` has one variant and every way of writing this warns — a `let ... else` as an
        // irrefutable pattern, a match as an infallible destructure. The lint is correct for one
        // build configuration and wrong for the other, so it is silenced rather than obeyed.
        #[allow(clippy::infallible_destructuring_match)]
        let client = match &self.backend {
            Backend::Http(client) => client,
            #[cfg(feature = "local-mt")]
            Backend::Local(_) => return self.run_local(lines, lang).await,
        };

        match self.style {
            Style::Chat => {
                let messages = prompt::translate(lines, lang, glossary);
                let response = client.complete(&messages).await?;
                Ok((prompt::parse_translation(&response, lines.len()), 1))
            }
            Style::Mt | Style::Local => {
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
                        let response = client.complete(&messages).await?;
                        Ok::<_, Error>(accept(&response, lang))
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
            // The in-process model translates one line at a time with every thread, so a chunk is
            // only how often progress reaches the file. Small, because this is the slow path and a
            // user who stops it half way should keep what it had done.
            Style::Local => 16,
        }
    }

    /// The in-process path: one line at a time, on a blocking thread.
    ///
    /// No concurrency, and that is the fast choice rather than the lazy one. `summo_mt` gives one
    /// decode every thread it has; running four at once on the same cores would divide the threads
    /// four ways and multiply the KV cache, for the same total throughput and four times the memory.
    ///
    /// `spawn_blocking` because a decode is seconds of unbroken CPU. On the async runtime it would
    /// stall every other request the daemon is serving — including the WebSocket carrying the
    /// transcript of a meeting being recorded right now.
    #[cfg(feature = "local-mt")]
    async fn run_local(&self, lines: &[&str], lang: &str) -> Result<(Vec<Option<String>>, usize)> {
        let Backend::Local(model) = &self.backend else {
            unreachable!("run_local is only reached with a local backend")
        };

        let model = model.clone();
        let lang = lang.to_string();
        let source = self.source.clone();
        let owned: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
        let count = owned.len();

        let out = tokio::task::spawn_blocking(move || {
            owned
                .iter()
                .map(|line| {
                    match model.translate(line, source.as_deref(), &lang) {
                        Ok(response) => response.filter(|text| plausible(text, &lang)),
                        // One line failing is that line, not the meeting. It stays untranslated,
                        // is counted as missing, and the rest still run.
                        Err(e) => {
                            tracing::warn!(error = %e, "a line failed to translate");
                            None
                        }
                    }
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| Error::Other(format!("the translation thread failed: {e}")))?;

        Ok((out, count))
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

/// The in-process translator named by the settings, loaded once per process.
///
/// Cached in a `OnceLock` keyed by nothing, because there is one translation model at a time and
/// it is hundreds of megabytes. The daemon builds a `Translator` per request; loading the weights
/// each time would spend a second and a gigabyte to translate one line.
///
/// A changed model id is therefore not picked up until the daemon restarts. That is a deliberate
/// trade and the settings screen says so — the alternative is holding two models resident to make
/// a setting nobody changes twice take effect a few seconds sooner.
#[cfg(feature = "local-mt")]
fn local_translator(
    paths: &Paths,
    settings: &Settings,
    mt: &summo_core::settings::Translator,
) -> Result<Translator> {
    use std::sync::{Arc, OnceLock};
    static LOADED: OnceLock<std::result::Result<Arc<LocalModel>, String>> = OnceLock::new();

    let id = mt.model.as_deref().unwrap_or("small100").to_string();
    let store = summo_models::ModelStore::new(paths.clone());
    let threads = settings.models.threads;

    let model = LOADED
        .get_or_init(move || {
            load_local(&store, &id, threads)
                .map(Arc::new)
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| Error::Other(e.clone()))?;

    Ok(Translator::local(
        model.clone(),
        settings.models.language.clone(),
    ))
}

/// Find the GGUF for `id` in the blob store and load it.
///
/// Goes through the registry rather than taking a path from the settings file, so a translation
/// model is installed by `summo pull` like everything else — content-addressed, resumable, sha256
/// checked — instead of being the one model a user has to place by hand.
#[cfg(feature = "local-mt")]
fn load_local(
    store: &summo_models::ModelStore,
    id: &str,
    threads: Option<usize>,
) -> Result<LocalModel> {
    // A path on disk is taken as one, before it is tried as a registry id.
    //
    // The registry is the right way in and this is deliberately the exception, for the case the
    // registry cannot serve: a model the user produced themselves. SMALL100's published export is
    // fp32 and 1.8 GB; quantized to int8 it is 449 MB and faster, and that file exists nowhere but
    // on the machine that made it. Without this, the smallest translation Summo can do is a number
    // in a document rather than something a user can run.
    let path = std::path::Path::new(id);
    if path.is_dir() {
        return load_seq2seq_dir(path, threads);
    }
    if path.is_file() {
        return Ok(LocalModel::Gguf(Box::new(summo_mt::Local::load(
            path, threads,
        )?)));
    }

    let model_id = summo_core::ModelId::parse(id).map_err(Error::Config)?;
    let manifest = store.installed(&model_id)?;

    // A speech model and a translation model are installed the same way and are not
    // interchangeable. Without this, pointing the translator at `gipformer-65m` fails inside
    // llama.cpp with a message about a file that is not a GGUF.
    if manifest.task != summo_models::Task::Translate {
        return Err(Error::Config(format!("`{id}` is not a translation model")));
    }

    let installed = store.resolve(&manifest)?;
    let file = |key: &str| {
        installed
            .param_path(key)
            .cloned()
            .ok_or_else(|| Error::InvalidManifest {
                id: id.to_string(),
                reason: format!("no `params.{key}`"),
            })
    };

    if manifest.runtime.contains("onnx") {
        let paths = summo_mt::Seq2SeqPaths {
            model: file("model")?,
            // Optional: a model published as one graph has no separate decoder, and one published
            // as a pair runs its encoder once a line instead of once a token.
            decoder: installed.param_path("decoder").cloned(),
            spm: file("spm")?,
            vocab: file("vocab")?,
        };
        Ok(LocalModel::Onnx(Box::new(
            summo_mt::Seq2Seq::load(&paths, threads)?.named(id),
        )))
    } else if manifest.runtime.contains("gguf") {
        Ok(LocalModel::Gguf(Box::new(
            summo_mt::Local::load(file("model")?, threads)?.named(id),
        )))
    } else {
        Err(Error::UnsupportedRuntime(manifest.runtime.clone()))
    }
}

/// A directory somebody exported themselves.
///
/// The quantized file is preferred when both are there, because that is the reason to have a
/// directory of one's own at all.
#[cfg(feature = "local-mt")]
fn load_seq2seq_dir(dir: &std::path::Path, threads: Option<usize>) -> Result<LocalModel> {
    let name = dir
        .file_name()
        .map_or_else(|| "local".to_string(), |n| n.to_string_lossy().into_owned());
    Ok(LocalModel::Onnx(Box::new(
        summo_mt::Seq2Seq::load(&summo_mt::seq2seq::discover(dir), threads)?.named(name),
    )))
}

/// Without the feature, the local option is a configuration that cannot be honoured — and saying so
/// beats translating with a model the user did not choose.
#[cfg(not(feature = "local-mt"))]
fn local_translator(
    _paths: &Paths,
    _settings: &Settings,
    _mt: &summo_core::settings::Translator,
) -> Result<Translator> {
    Err(Error::Config(
        "this build cannot run a translation model in-process; point the translator at an \
         endpoint, or use a build with the `local-mt` feature"
            .into(),
    ))
}

/// Take a model's reply, or refuse it.
///
/// Two things can be wrong with an answer that parsed perfectly well, and both are silent:
///
/// * It kept talking past the sentence, and everything after the first line is the model's own
///   continuation. [`prompt::parse_mt`] cuts that.
/// * It answered in the wrong language. Measured: MiLMMT-46-1B, given a Vietnamese line and asked
///   for Japanese, returned fluent **Thai** — at temperature zero, reproducibly, in two different
///   quantizations. A wrong language is not a malformed response, so nothing downstream can see it.
///
/// Refused rather than retried. `None` leaves the original line in place, which the user can read
/// and check, and the outcome already reports it as missing.
fn accept(response: &str, lang: &str) -> Option<String> {
    prompt::parse_mt(response).filter(|text| plausible(text, lang))
}

/// Whether a reply is in the language it was asked for.
///
/// Split from [`accept`] because the in-process path has already parsed its own output — a seq2seq
/// model generates only the translation, so there is no continuation to cut — and would otherwise
/// have to run the parser twice to reach this check.
fn plausible(text: &str, lang: &str) -> bool {
    let ok = summo_llm::lang::plausible(text, lang);
    if !ok {
        tracing::warn!(
            %lang,
            "a translated line came back in another language; keeping the original"
        );
    }
    ok
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
