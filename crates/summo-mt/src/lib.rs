//! A language model running in this process.
//!
//! Everything else in Summo that touches a language model talks HTTP to something the user
//! started — Ollama, llama.cpp, LM Studio, or a hosted API. That is the right default for
//! summaries, where the model is large and the choice is genuinely the user's. It is the wrong
//! default for **translation**, because translation is the one text feature that can be free:
//! a 1B translation model is 800 MB, runs on a CPU in under a second a line, and beats a general
//! 8B model at the job.
//!
//! Requiring a second program to be installed and running is what stops that being true in
//! practice. "Install Ollama, pull a model, keep it running" is three steps between a user and a
//! feature that costs nothing — and on a machine that already has Summo, all three are avoidable.
//! So this crate loads a GGUF and answers in-process, and translation works with nothing else
//! installed.
//!
//! ## What this is not
//!
//! Not a translator. This is a *runtime*: a prompt goes in, text comes out. The translation
//! template lives in `summo-llm` beside the one the HTTP path uses, so the two cannot drift into
//! prompting the same model differently. Keeping the layers apart is also what lets summaries and
//! chat move in-process later without this crate learning what a meeting is.
//!
//! Not a replacement for the HTTP path. Someone already running Ollama should keep using it, and
//! someone who wants a hosted model must be able to have one. This is the third option, and the
//! only one that needs nothing installed.
//!
//! ## Cost of the feature
//!
//! Behind the `local` feature, because it compiles llama.cpp — the same shape of dependency as
//! `sherpa-rs` in `summo-asr`, and the same reason for the gate: a build that only needs the
//! registry client should not pay a C++ compile for it.

#![cfg(feature = "local")]

use std::{
    num::NonZeroU32,
    path::Path,
    sync::{Mutex, OnceLock},
};

use llama_cpp_2::{
    context::params::LlamaContextParams,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel, params::LlamaModelParams},
    sampling::LlamaSampler,
};
use summo_core::{Error, Result};

/// Context window.
///
/// A meeting utterance is a sentence; 512 tokens is several. It is deliberately small because the
/// KV cache is allocated against it — asking for the model's full window would spend hundreds of
/// megabytes to hold one line of speech.
const CONTEXT: u32 = 512;

/// Ceiling on generated tokens.
///
/// A translation is about as long as its input. This is the backstop for a model that has decided
/// not to stop, which is a thing small models do; the newline check below is what normally ends a
/// generation.
const MAX_TOKENS: usize = 256;

/// llama.cpp's global init, done once per process.
///
/// Not per model: the second call fails, and a user who switches translation model in settings
/// would otherwise be unable to load the new one without restarting.
fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<std::result::Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            let mut backend = LlamaBackend::init().map_err(|e| e.to_string())?;
            // llama.cpp writes a paragraph of load diagnostics to stderr. Summo's log is read by
            // users when something is wrong, and burying the one line that matters under a tensor
            // inventory is how a log stops being read at all.
            backend.void_logs();
            Ok(backend)
        })
        .as_ref()
        .map_err(|e| Error::Other(format!("cannot start the local model runtime: {e}")))
}

/// A GGUF model, loaded and ready.
///
/// One generation at a time, with every thread, rather than several at once with a share each. On
/// a CPU these are the same cores either way and llama.cpp parallelises a single decode well, so
/// splitting into parallel contexts would be slower *and* multiply the KV cache. The concurrency
/// the HTTP path gets from a server's slots is deliberately not reproduced here.
///
/// `Debug` prints the model's name and nothing else. The alternative — deriving it — would put a
/// few hundred megabytes of loaded weights into any log line that formatted a struct holding one.
pub struct Local {
    model: LlamaModel,
    /// Held for the whole of a generation. Two callers interleaving tokens into one KV cache would
    /// corrupt both answers.
    turn: Mutex<()>,
    threads: i32,
    name: String,
}

impl std::fmt::Debug for Local {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Local")
            .field("model", &self.name)
            .field("threads", &self.threads)
            .finish()
    }
}

impl Local {
    /// Load a GGUF from disk.
    ///
    /// `threads` of `None` follows the machine, capped: past about eight, a 1B model on a CPU stops
    /// getting faster and starts contending with everything else the user is doing — including the
    /// speech recognition that is the reason the app is open.
    pub fn load(path: impl AsRef<Path>, threads: Option<usize>) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(Error::Other(format!(
                "no translation model at {}",
                path.display()
            )));
        }

        let backend = backend()?;
        let model = LlamaModel::load_from_file(backend, path, &LlamaModelParams::default())
            .map_err(|e| Error::Other(format!("cannot load {}: {e}", path.display())))?;

        let threads = threads
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get() / 2))
            .clamp(1, 8);

        Ok(Self {
            model,
            turn: Mutex::new(()),
            threads: i32::try_from(threads).unwrap_or(4),
            name: path
                .file_stem()
                .map_or_else(|| "local".to_string(), |n| n.to_string_lossy().into_owned()),
        })
    }

    /// Call this model something a reader will recognise.
    ///
    /// The default is the file's stem, which is right for a GGUF somebody points at by hand and
    /// useless for one installed through the registry: the blob store is content-addressed, so the
    /// file is named after its sha256. Without this, every translation file was stamped
    /// `model:74d38ba7…`, which answers "which model wrote this" with a number nobody can look up.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !name.trim().is_empty() {
            self.name = name;
        }
        self
    }

    /// What to call this model in a translation file's header.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Continue `prompt`, greedily, stopping at a newline.
    ///
    /// Greedy — never sampled. A translation model has one right answer for a sentence, and
    /// sampling is what made a 1B model drift into a language nobody asked for. It also makes a
    /// re-translation reproducible, which is what lets a user re-run one and compare.
    ///
    /// Stopping at a newline is not a formatting preference. These models continue past the
    /// sentence they were asked for — another `English:` turn, the source repeated, a line of
    /// commentary — and everything after the first line is the model talking to itself.
    ///
    /// A fresh context per call, so nothing survives from the previous line. Reusing one and
    /// clearing it would be marginally faster and would put the correctness of every translation
    /// on remembering to clear it.
    pub fn complete(&self, prompt: &str) -> Result<String> {
        let _turn = self
            .turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let backend = backend()?;
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(CONTEXT))
            .with_n_batch(CONTEXT)
            .with_n_threads(self.threads)
            .with_n_threads_batch(self.threads);
        let mut context = self
            .model
            .new_context(backend, params)
            .map_err(|e| Error::Other(format!("cannot open a context: {e}")))?;

        let tokens = self
            .model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| Error::Other(format!("cannot tokenize the prompt: {e}")))?;
        if tokens.len() >= CONTEXT as usize {
            // One utterance longer than the context is a recognition failure upstream, not a
            // translation problem. Saying so beats a truncated translation that looks complete.
            return Err(Error::Other(format!(
                "the line is {} tokens, longer than the {CONTEXT}-token window",
                tokens.len()
            )));
        }

        let mut batch = LlamaBatch::new(CONTEXT as usize, 1);
        let last = tokens.len() - 1;
        for (i, token) in tokens.iter().enumerate() {
            // Logits only for the final position: the rest are prompt, and asking for all of them
            // allocates a vocabulary-sized row per token for no reason.
            batch
                .add(*token, i as i32, &[0], i == last)
                .map_err(|e| Error::Other(format!("cannot build the prompt batch: {e}")))?;
        }
        context
            .decode(&mut batch)
            .map_err(|e| Error::Other(format!("cannot read the prompt: {e}")))?;

        let mut sampler = LlamaSampler::greedy();
        // Bytes, not a `String`, and this is the difference between working in Japanese and not.
        // A token is a piece of UTF-8, not a character: one Japanese character routinely spans two
        // tokens, and decoding each token to a string on its own turns the pair into two
        // replacement characters. Accumulating the bytes and decoding once at the end is the only
        // version that is correct for the languages this feature exists for.
        let mut out: Vec<u8> = Vec::new();
        let start = i32::try_from(tokens.len()).unwrap_or(i32::MAX);

        for position in start..start.saturating_add(i32::try_from(MAX_TOKENS).unwrap_or(0)) {
            let token = sampler.sample(&context, -1);
            if self.model.is_eog_token(token) {
                break;
            }
            sampler.accept(token);

            let piece = self
                .model
                .token_to_piece_bytes(token, 32, false, None)
                .unwrap_or_default();
            if let Some(cut) = piece.iter().position(|b| *b == b'\n') {
                // Keep what came before the break. A model that ends its answer and opens a new
                // turn inside one token has still answered.
                out.extend_from_slice(&piece[..cut]);
                break;
            }
            out.extend_from_slice(&piece);

            batch.clear();
            batch
                .add(token, position, &[0], true)
                .map_err(|e| Error::Other(format!("cannot continue: {e}")))?;
            context
                .decode(&mut batch)
                .map_err(|e| Error::Other(format!("cannot generate: {e}")))?;
        }

        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GGUF to test against. Skipped without one, so the suite runs on a machine that has not
    /// downloaded a model.
    fn model_path() -> Option<std::path::PathBuf> {
        std::env::var_os("SUMMO_TEST_GGUF").map(std::path::PathBuf::from)
    }

    /// The blob store names a file after its hash, so the path is not a name. A translation file
    /// header saying `model:74d38ba7…` tells a reader nothing they can act on.
    #[test]
    fn a_model_can_be_called_something_a_reader_recognises() {
        let Some(path) = model_path() else {
            eprintln!("skipping: set SUMMO_TEST_GGUF");
            return;
        };
        let mt = Local::load(&path, Some(1)).unwrap().named("milmmt-46-1b");
        assert_eq!(mt.name(), "milmmt-46-1b");
        assert_ne!(
            Local::load(&path, Some(1)).unwrap().named("  ").name(),
            "  ",
            "an empty name must not replace a real one"
        );
    }

    /// The error a user is most likely to see: a path in settings that no longer has a file at it.
    /// It has to name the path, because the whole diagnosis is "that file moved".
    #[test]
    fn a_missing_model_says_which_path_was_tried() {
        let err = Local::load("/nonexistent/model.gguf", None).unwrap_err();
        assert!(
            err.to_string().contains("/nonexistent/model.gguf"),
            "got: {err}"
        );
    }

    #[test]
    fn a_directory_is_not_a_model() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Local::load(dir.path(), None).is_err());
    }

    #[test]
    fn it_translates() {
        let Some(path) = model_path() else {
            eprintln!("skipping: set SUMMO_TEST_GGUF to a translation model");
            return;
        };
        let mt = Local::load(&path, Some(8)).unwrap();
        let out = mt
            .complete(
                "Translate this from Vietnamese to English:\n\
                 Vietnamese: Chiều nay mình chốt lại spec API.\n\
                 English:",
            )
            .unwrap();
        assert!(!out.is_empty(), "expected a translation");
        assert!(!out.contains('\n'), "one line only, got `{out}`");
        assert!(
            out.to_lowercase().contains("api"),
            "expected the term to survive, got `{out}`"
        );
    }

    /// Greedy decoding, so the same line twice is the same answer. A user who re-runs a translation
    /// to compare it with the last one is otherwise comparing the model's mood.
    #[test]
    fn the_same_line_translates_the_same_way_twice() {
        let Some(path) = model_path() else {
            eprintln!("skipping: set SUMMO_TEST_GGUF");
            return;
        };
        let mt = Local::load(&path, Some(4)).unwrap();
        let prompt = "Translate this from Vietnamese to English:\nVietnamese: Xin chào.\nEnglish:";
        assert_eq!(mt.complete(prompt).unwrap(), mt.complete(prompt).unwrap());
    }

    /// The bug that only shows up in the languages this feature exists for: a Japanese character
    /// routinely spans two tokens, and decoding each token to a `String` on its own turns the pair
    /// into replacement characters. The output has to be assembled as bytes.
    #[test]
    fn japanese_comes_back_as_japanese_and_not_as_replacement_characters() {
        let Some(path) = model_path() else {
            eprintln!("skipping: set SUMMO_TEST_GGUF");
            return;
        };
        let mt = Local::load(&path, Some(8)).unwrap();
        let out = mt
            .complete(
                "Translate this from Vietnamese to Japanese:\n\
                 Vietnamese: Cảm ơn bạn rất nhiều.\n\
                 Japanese:",
            )
            .unwrap();
        assert!(!out.contains('\u{fffd}'), "mojibake in `{out}`");
        assert!(
            out.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c)),
            "expected kana, got `{out}`"
        );
    }

    /// Two lines through one loaded model must not influence each other. This is the test that
    /// fails if state is ever carried between generations.
    #[test]
    fn one_line_does_not_leak_into_the_next() {
        let Some(path) = model_path() else {
            eprintln!("skipping: set SUMMO_TEST_GGUF");
            return;
        };
        let mt = Local::load(&path, Some(4)).unwrap();
        let first = "Translate this from Vietnamese to English:\nVietnamese: Xin chào.\nEnglish:";
        let alone = mt.complete(first).unwrap();

        mt.complete("Translate this from Vietnamese to Japanese:\nVietnamese: Cảm ơn.\nJapanese:")
            .unwrap();
        assert_eq!(
            mt.complete(first).unwrap(),
            alone,
            "a previous line changed the answer to this one"
        );
    }
}
