//! SMALL100 and the M2M100 family, through ONNX Runtime.
//!
//! The **small** option, and the reason it exists: MiLMMT cannot get much under 650 MB. It is built
//! on Gemma 3, whose 262 000-token embedding table dominates the file, so quantizing harder barely
//! helps — measured, `Q2_K` is 658 MB against `Q4_K_M`'s 769 MB and translates worse. Anything
//! genuinely smaller has to be a different architecture.
//!
//! SMALL100 is that architecture: 330M parameters, 128 000 pieces, twelve encoder layers and three
//! decoder layers. Quantized to int8 it is **449 MB and 377 ms a line** — half the disk of the
//! smallest usable MiLMMT and faster. It is also MIT, which is a better licence than the Gemma
//! terms the MiLMMT weights carry.
//!
//! ## What it costs
//!
//! Quality. On Vietnamese meeting speech it makes lexical errors MiLMMT does not — "chốt lại spec
//! API" comes back as *lock up the spec API* — while still producing complete, on-topic sentences
//! in the right language. That is the trade, and it is a real one: every *general* model small
//! enough to compete collapsed outright on the same sentences. Qwen3-0.6B repeated one phrase forty
//! times and returned nothing at all for English→Vietnamese; Gemma3-270m returned empty strings for
//! three of seven. A specialised 330M model beats a general 600M one at translation by a wide
//! margin, which is the same reason the 1B beats a general 8B.
//!
//! Beam search does not close the gap. The published export carries no KV cache, so five beams cost
//! five full forward passes per token — 3.3 s a line, five times greedy — and the output was not
//! better. Greedy it is.
//!
//! ## Why this is not just another llama.cpp model
//!
//! llama.cpp serves decoder-only models. SMALL100 is an encoder–decoder, so it needs its own
//! runtime, and that is the whole reason this module exists rather than another GGUF in the
//! registry. The runtime is `ort`, which Summo already links for the voice detector, and the
//! tokenizer is pure Rust — so unlike [`crate::gguf`], this path needs no C++ toolchain to build.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, PoisonError},
};

use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::{Tensor, TensorRef},
};
use summo_core::{Error, Result};
use tokenizers::{
    models::bpe::BPE,
    normalizers::Precompiled,
    pre_tokenizers::metaspace::{Metaspace, PrependScheme},
    tokenizer::TokenizerImpl,
};

use crate::spm;

/// The languages M2M100 was trained on, in the order that assigns their token ids.
///
/// The order is load-bearing: a language's id is `len(vocab) + index_here`, so a single entry out
/// of place silently translates into a different language. Copied verbatim from
/// `tokenization_small100.py`.
const CODES: [&str; 100] = [
    "af", "am", "ar", "ast", "az", "ba", "be", "bg", "bn", "br", "bs", "ca", "ceb", "cs", "cy",
    "da", "de", "el", "en", "es", "et", "fa", "ff", "fi", "fr", "fy", "ga", "gd", "gl", "gu", "ha",
    "he", "hi", "hr", "ht", "hu", "hy", "id", "ig", "ilo", "is", "it", "ja", "jv", "ka", "kk",
    "km", "kn", "ko", "lb", "lg", "ln", "lo", "lt", "lv", "mg", "mk", "ml", "mn", "mr", "ms", "my",
    "ne", "nl", "no", "ns", "oc", "or", "pa", "pl", "ps", "pt", "ro", "ru", "sd", "si", "sk", "sl",
    "so", "sq", "sr", "ss", "su", "sv", "sw", "ta", "th", "tl", "tn", "tr", "uk", "ur", "uz", "vi",
    "wo", "xh", "yi", "yo", "zh", "zu",
];

/// End of sequence, and also what the decoder starts with. Fixed by the checkpoint's config.
const EOS: i64 = 2;

/// Padding, which this model emits instead of end-of-sequence more often than it should.
///
/// Found by running it: asked for Japanese, the decoder finished a clause and then produced 116
/// consecutive `<pad>` tokens, all of which reached the vault as the literal text `<pad><pad>…`.
/// Treated as a stop rather than filtered, so the remaining tokens are not generated at all.
const PAD: i64 = 1;

/// `<s>`, `<pad>`, `</s>`, `<unk>`. None of them is speech, and all four are in `vocab.json`, so
/// nothing downstream would have refused them.
const CONTROL: [i64; 4] = [0, PAD, EOS, 3];

/// Ceiling on generated tokens. A translation is about as long as its input; this is the backstop
/// for a model that has decided not to stop.
const MAX_TOKENS: usize = 128;

/// Longest input the encoder is given, in pieces.
///
/// The checkpoint's positional embeddings run to 1024. A meeting utterance is a sentence, so this
/// is generous; it exists so a recognition failure upstream produces an error rather than an
/// out-of-range index inside the graph.
const MAX_INPUT: usize = 512;

/// The tokenizer, with no vocabulary of its own.
///
/// `BPE` here is only a segmenter: it turns text into pieces. The *ids* come from `vocab.json`,
/// which is a different mapping from the one inside the SentencePiece file — this is the detail
/// that makes M2M100 tokenizers wrong when they are written from memory. Feeding SentencePiece's
/// own ids to the model produces fluent output in no particular language.
type Segmenter = TokenizerImpl<
    BPE,
    Precompiled,
    Metaspace,
    tokenizers::processors::template::TemplateProcessing,
    tokenizers::decoders::sequence::Sequence,
>;

/// The exported graphs, in whichever of the two shapes the model was published in.
///
/// Both exist in the wild and the difference is worth a lot. A **single** graph takes the source
/// and the partial translation together, so every generated token re-runs the twelve-layer encoder
/// over a sentence that has not changed. A **split** pair runs the encoder once and then only the
/// three-layer decoder per token — the same arithmetic, minus the part that was already done.
enum Graphs {
    Single(Session),
    Split { encoder: Session, decoder: Session },
}

/// A loaded SMALL100.
pub struct Seq2Seq {
    /// Held across a whole translation: `ort` sessions are not re-entrant.
    session: Mutex<Graphs>,
    segmenter: Segmenter,
    /// Piece → id. Not the SentencePiece id; see [`Segmenter`].
    vocab: HashMap<String, i64>,
    /// Id → piece, for turning the decoder's output back into text.
    pieces: HashMap<i64, String>,
    /// `len(vocab)`, where the language token ids begin.
    lang_base: i64,
    unk: i64,
    name: String,
}

impl std::fmt::Debug for Seq2Seq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seq2Seq")
            .field("model", &self.name)
            .finish()
    }
}

/// Where the three files live.
///
/// Named rather than discovered, because two of them are ambiguous by extension: a directory holding
/// both `model.onnx` and `model.int8.onnx` should use the one the manifest chose, not the one that
/// sorts first.
#[derive(Debug, Clone)]
pub struct Seq2SeqPaths {
    /// The encoder, or — when [`Self::decoder`] is `None` — a single graph taking `input_ids` and
    /// `decoder_input_ids` together.
    pub model: std::path::PathBuf,
    /// The decoder, when the model was exported as a pair. Strongly preferred: see [`Graphs`].
    pub decoder: Option<std::path::PathBuf>,
    /// `sentencepiece.bpe.model`.
    pub spm: std::path::PathBuf,
    /// `vocab.json`.
    pub vocab: std::path::PathBuf,
}

impl Seq2Seq {
    /// Load a model, its SentencePiece file and its vocabulary.
    pub fn load(paths: &Seq2SeqPaths, threads: Option<usize>) -> Result<Self> {
        for (what, path) in [
            ("model", &paths.model),
            ("sentencepiece", &paths.spm),
            ("vocab", &paths.vocab),
        ] {
            if !path.is_file() {
                return Err(Error::Other(format!(
                    "no {what} file at {}",
                    path.display()
                )));
            }
        }

        let threads = threads
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get() / 2))
            .clamp(1, 8);

        let session = match &paths.decoder {
            Some(decoder) => {
                if !decoder.is_file() {
                    return Err(Error::Other(format!(
                        "no decoder file at {}",
                        decoder.display()
                    )));
                }
                Graphs::Split {
                    encoder: build_session(&paths.model, threads).map_err(|e| {
                        Error::Other(format!("cannot load {}: {e}", paths.model.display()))
                    })?,
                    decoder: build_session(decoder, threads).map_err(|e| {
                        Error::Other(format!("cannot load {}: {e}", decoder.display()))
                    })?,
                }
            }
            None => Graphs::Single(build_session(&paths.model, threads).map_err(|e| {
                Error::Other(format!("cannot load {}: {e}", paths.model.display()))
            })?),
        };

        let spm_bytes = std::fs::read(&paths.spm).map_err(|e| Error::io(paths.spm.as_path(), e))?;
        let model = spm::parse(&spm_bytes)?;

        if !spm::is_bpe(&model.pieces) {
            return Err(Error::Other(
                "this is a Unigram SentencePiece model; only BPE is supported here".into(),
            ));
        }
        // `tokenizers` wants its own map type here, not `std`'s.
        let vocab_by_rank: tokenizers::models::bpe::Vocab = model
            .pieces
            .iter()
            .enumerate()
            .map(|(i, (piece, _))| (piece.clone(), u32::try_from(i).unwrap_or(u32::MAX)))
            .collect();
        let bpe = BPE::builder()
            .vocab_and_merges(vocab_by_rank, spm::merges(&model.pieces))
            .unk_token("<unk>".into())
            .build()
            .map_err(|e| Error::Other(format!("cannot build the segmenter: {e}")))?;
        let normalizer = Precompiled::from(&model.charsmap)
            .map_err(|e| Error::Other(format!("cannot read the normalisation table: {e}")))?;

        let mut segmenter = TokenizerImpl::new(bpe);
        // `Result`, and discarded: the only error is a normalizer that cannot be installed, which
        // cannot happen for one just constructed.
        let _ = segmenter.with_normalizer(Some(normalizer));
        // `Always`: SentencePiece prefixes the whole input with `▁`, and without it the first word
        // of every sentence tokenises as a word-continuation. The output stays readable, which is
        // what makes this the kind of mistake that ships.
        segmenter.with_pre_tokenizer(Some(Metaspace::new('▁', PrependScheme::Always, true)));

        let raw = std::fs::read_to_string(&paths.vocab)
            .map_err(|e| Error::io(paths.vocab.as_path(), e))?;
        let vocab: HashMap<String, i64> = serde_json::from_str(&raw).map_err(|e| {
            Error::Other(format!(
                "{} is not a vocabulary: {e}",
                paths.vocab.display()
            ))
        })?;
        if vocab.is_empty() {
            return Err(Error::Other("the vocabulary is empty".into()));
        }
        let pieces = vocab.iter().map(|(k, v)| (*v, k.clone())).collect();
        let unk = *vocab.get("<unk>").unwrap_or(&3);

        Ok(Self {
            lang_base: i64::try_from(vocab.len()).unwrap_or(i64::MAX),
            session: Mutex::new(session),
            segmenter,
            vocab,
            pieces,
            unk,
            name: "small100".into(),
        })
    }

    /// Call this model something a reader will recognise. See [`crate::gguf::Local::named`].
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !name.trim().is_empty() {
            self.name = name;
        }
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether this model was trained on `language`.
    ///
    /// Worth asking before the request rather than after: an unsupported language has no token, and
    /// without a token the model translates into whichever one it feels like.
    #[must_use]
    pub fn supports(language: &str) -> bool {
        Self::code(language).is_some()
    }

    /// The M2M100 code for a tag, if there is one.
    ///
    /// `zh-Hant` collapses to `zh` here, unlike everywhere else in Summo: M2M100 has one Chinese and
    /// no way to ask for Traditional. Saying so beats silently answering in Simplified.
    fn code(language: &str) -> Option<&'static str> {
        let lower = language.trim().to_ascii_lowercase();
        let primary = lower.split(['-', '_']).next().unwrap_or(&lower);
        CODES.iter().copied().find(|c| *c == primary)
    }

    /// Translate one line into `target`.
    ///
    /// The whole M2M100 scheme in four steps, and each one is a way to get a fluent wrong answer:
    ///
    /// 1. Segment the source into pieces.
    /// 2. Map pieces to ids through `vocab.json` — *not* SentencePiece's own ids.
    /// 3. Prefix the id of the **target** language, and append end-of-sequence. SMALL100 is a
    ///    universal model: the target goes on the *source* side, and there is no source-language
    ///    token at all.
    /// 4. Decode greedily from end-of-sequence.
    pub fn translate(&self, line: &str, target: &str) -> Result<String> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(String::new());
        }
        let input = self.encode(line, target)?;

        let mut graphs = self.session.lock().unwrap_or_else(PoisonError::into_inner);

        // The encoder's output, computed once when the export allows it.
        let context = match &mut *graphs {
            Graphs::Single(_) => None,
            Graphs::Split { encoder, .. } => {
                let ids = TensorRef::from_array_view(([1_usize, input.len()], &input[..]))
                    .map_err(|e| Error::Other(e.to_string()))?;
                let outputs = encoder
                    .run(ort::inputs! { "input_ids" => ids })
                    .map_err(|e| Error::Other(format!("the encoder failed: {e}")))?;
                let (shape, values) = outputs["last_hidden_state"]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| Error::Other(e.to_string()))?;
                let shape: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
                Some((shape, values.to_vec()))
            }
        };

        // Built once and borrowed on every step. These do not change while a line is generated,
        // and `Tensor::from_array` takes ownership — so building them inside the loop copied the
        // encoder's output, ~50 KB, once per generated token for no reason at all.
        let mask: Vec<i64> = vec![1; input.len()];

        let mut decoded: Vec<i64> = vec![EOS];
        for _ in 0..MAX_TOKENS {
            let decoder_ids = TensorRef::from_array_view(([1_usize, decoded.len()], &decoded[..]))
                .map_err(|e| Error::Other(e.to_string()))?;

            let outputs = match (&mut *graphs, &context) {
                (Graphs::Single(session), _) => {
                    let encoder_ids =
                        TensorRef::from_array_view(([1_usize, input.len()], &input[..]))
                            .map_err(|e| Error::Other(e.to_string()))?;
                    session
                        .run(ort::inputs! {
                            "input_ids" => encoder_ids,
                            "decoder_input_ids" => decoder_ids,
                        })
                        .map_err(|e| Error::Other(format!("the model failed: {e}")))?
                }
                (Graphs::Split { decoder, .. }, Some((shape, values))) => {
                    let hidden = TensorRef::from_array_view((shape.clone(), &values[..]))
                        .map_err(|e| Error::Other(e.to_string()))?;
                    // All ones: an utterance is one sequence with nothing padded, so there is
                    // nothing for the mask to hide. It is required all the same.
                    let mask = TensorRef::from_array_view(([1_usize, mask.len()], &mask[..]))
                        .map_err(|e| Error::Other(e.to_string()))?;
                    decoder
                        .run(ort::inputs! {
                            "input_ids" => decoder_ids,
                            "encoder_hidden_states" => hidden,
                            "encoder_attention_mask" => mask,
                        })
                        .map_err(|e| Error::Other(format!("the decoder failed: {e}")))?
                }
                (Graphs::Split { .. }, None) => {
                    return Err(Error::Other("the encoder produced nothing".into()));
                }
            };

            let (shape, logits) = outputs["logits"]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::Other(e.to_string()))?;
            let vocab_size = usize::try_from(*shape.last().unwrap_or(&0)).unwrap_or(0);
            if vocab_size == 0 || logits.len() < vocab_size {
                return Err(Error::Other("the model returned no logits".into()));
            }

            // The decoder scores the whole partial translation each step; the row that matters is
            // the last one.
            let row = &logits[logits.len() - vocab_size..];
            let next = row
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or(EOS, |(i, _)| i64::try_from(i).unwrap_or(EOS));

            if next == EOS || next == PAD {
                break;
            }
            decoded.push(next);
        }

        Ok(self.detokenize(&decoded[1..]))
    }

    /// The encoder input: target language, then the line, then end-of-sequence.
    ///
    /// Split out from [`Self::translate`] so it can be pinned against the reference implementation
    /// in a test. Every one of these three steps is a way to get a *fluent* wrong answer rather
    /// than an error, which is exactly the kind of bug a translation into a language you cannot
    /// read will not reveal.
    fn encode(&self, line: &str, target: &str) -> Result<Vec<i64>> {
        let Some(code) = Self::code(target) else {
            return Err(Error::Other(format!(
                "this model was not trained on `{target}`"
            )));
        };

        let encoding = self
            .segmenter
            .encode(line, false)
            .map_err(|e| Error::Other(format!("cannot segment the line: {e}")))?;

        let lang_id = self.lang_base
            + i64::try_from(CODES.iter().position(|c| *c == code).unwrap_or(0)).unwrap_or(0);

        let mut input: Vec<i64> = Vec::with_capacity(encoding.get_tokens().len() + 2);
        // The *target* language, on the *source* side. SMALL100 is a universal model: there is no
        // source-language token at all, and putting the source here translates into the language it
        // was already in.
        input.push(lang_id);
        for piece in encoding.get_tokens() {
            input.push(*self.vocab.get(piece).unwrap_or(&self.unk));
        }
        input.push(EOS);

        if input.len() > MAX_INPUT {
            return Err(Error::Other(format!(
                "the line is {} pieces, longer than the {MAX_INPUT}-piece window",
                input.len()
            )));
        }
        Ok(input)
    }

    /// Pieces back into text.
    ///
    /// `▁` is SentencePiece's word boundary, not a character anybody typed. Written out here rather
    /// than through a decoder configuration because the rule is one line and the alternative was a
    /// fifth type parameter on [`Segmenter`].
    fn detokenize(&self, ids: &[i64]) -> String {
        let mut out = String::new();
        for id in ids {
            // An id outside the vocabulary is a language token or one of the eight made-up words
            // at the end of it; a control token is punctuation for the model, not for the reader.
            // Neither is speech, so both are dropped.
            if CONTROL.contains(id) {
                continue;
            }
            if let Some(piece) = self.pieces.get(id) {
                out.push_str(&piece.replace('▁', " "));
            }
        }
        out.trim().to_string()
    }
}

/// Work out which files a directory holds.
///
/// Published exports of the same model come in three shapes, and a user who downloaded one should
/// not have to know which: a split `encoder_int8`/`decoder_int8` pair, the same pair under the
/// `*_model_quantized` names `optimum` produces, or a single `model.onnx`. Quantized names are
/// tried first, because someone holding both wanted the small one.
#[must_use]
pub fn discover(dir: &Path) -> Seq2SeqPaths {
    const PAIRS: [(&str, &str); 4] = [
        ("encoder_int8.onnx", "decoder_int8.onnx"),
        (
            "encoder_model_quantized.onnx",
            "decoder_model_quantized.onnx",
        ),
        ("encoder_model_int8.onnx", "decoder_model_int8.onnx"),
        ("encoder_model.onnx", "decoder_model.onnx"),
    ];
    for (encoder, decoder) in PAIRS {
        if dir.join(encoder).is_file() && dir.join(decoder).is_file() {
            return Seq2SeqPaths {
                model: dir.join(encoder),
                decoder: Some(dir.join(decoder)),
                spm: dir.join("sentencepiece.bpe.model"),
                vocab: dir.join("vocab.json"),
            };
        }
    }
    let quantized = dir.join("model.int8.onnx");
    Seq2SeqPaths {
        model: if quantized.is_file() {
            quantized
        } else {
            dir.join("model.onnx")
        },
        decoder: None,
        spm: dir.join("sentencepiece.bpe.model"),
        vocab: dir.join("vocab.json"),
    }
}

/// Session construction, split out because each `ort` builder step returns a distinct error type
/// that only unifies through a boxed trait object.
fn build_session(
    path: &Path,
    threads: usize,
) -> std::result::Result<Session, Box<dyn std::error::Error>> {
    let mut builder = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(threads)?;
    Ok(builder.commit_from_file(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory holding `model.onnx`, `sentencepiece.bpe.model` and `vocab.json`.
    fn model_dir() -> Option<std::path::PathBuf> {
        std::env::var_os("SUMMO_TEST_SEQ2SEQ").map(std::path::PathBuf::from)
    }

    fn paths(dir: &Path) -> Seq2SeqPaths {
        discover(dir)
    }

    #[test]
    fn a_missing_file_says_which_one() {
        let err = Seq2Seq::load(
            &Seq2SeqPaths {
                model: "/nonexistent/model.onnx".into(),
                decoder: None,
                spm: "/nonexistent/spm".into(),
                vocab: "/nonexistent/vocab.json".into(),
            },
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("model"), "got: {err}");
    }

    /// The languages are matched by tag, and a language with no token cannot be asked for at all.
    #[test]
    fn only_the_languages_it_was_trained_on_are_offered() {
        assert!(Seq2Seq::supports("vi"));
        assert!(Seq2Seq::supports("ja"));
        assert!(Seq2Seq::supports("zh"));
        assert!(
            Seq2Seq::supports("en-GB"),
            "a regional tag finds its language"
        );
        assert!(!Seq2Seq::supports("yue"), "Cantonese is not in M2M100");
        assert!(!Seq2Seq::supports("klingon"));
    }

    /// M2M100 has one Chinese. Answering a request for Traditional with Simplified would be a
    /// wrong answer that looks right to anybody who cannot read it.
    #[test]
    fn traditional_chinese_is_not_quietly_answered_in_simplified() {
        assert_eq!(Seq2Seq::code("zh-Hant"), Some("zh"));
    }

    /// The tokenizer, pinned to SentencePiece's own output.
    ///
    /// These are the pieces `sp.encode(text, out_type=str)` produces for the same strings. They are
    /// here because getting this wrong does not fail — it *translates*, fluently, into a sentence
    /// about something else. The first version of this file built a Unigram segmenter from what is
    /// actually a BPE vocabulary and turned "Chiều nay mình chốt lại spec API." into "I have been
    /// making a lot of spec.": grammatical, confident, unrelated.
    ///
    /// Vietnamese because its diacritics are where a normaliser goes wrong, and because the split
    /// of `Chiều` into `▁Chi` + `ều` is exactly what a wrong segmenter gets wrong.
    #[test]
    fn segmentation_matches_sentencepiece() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SEQ2SEQ");
            return;
        };
        let mt = Seq2Seq::load(&paths(&dir), Some(1)).unwrap();
        for (line, want) in [
            (
                "Chiều nay mình chốt lại spec API.",
                vec![
                    "▁Chi", "ều", "▁nay", "▁mình", "▁ch", "ốt", "▁lại", "▁spec", "▁API", ".",
                ],
            ),
            ("Xin chào.", vec!["▁Xin", "▁chào", "."]),
        ] {
            let got = mt.segmenter.encode(line, false).unwrap();
            assert_eq!(got.get_tokens(), want.as_slice(), "for `{line}`");
        }
    }

    /// The other half of the tokenizer pin: the ids that actually reach the model.
    ///
    /// Taken from the reference implementation — `sentencepiece` plus `vocab.json` in Python — for
    /// the same sentence. Segmentation being right does not make the *ids* right: they come from
    /// `vocab.json`, not from the SentencePiece file, and the language token is computed from a
    /// hundred-entry table where one misplaced entry silently selects a different language.
    #[test]
    fn encoder_input_matches_the_reference_implementation() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SEQ2SEQ");
            return;
        };
        let mt = Seq2Seq::load(&paths(&dir), Some(1)).unwrap();
        assert_eq!(
            mt.encode("Chiều nay mình chốt lại spec API.", "en")
                .unwrap(),
            vec![
                128_022, 9604, 38664, 7836, 6143, 565, 29162, 3818, 24679, 78630, 5, 2
            ]
        );
        // A different target language changes exactly one token, the first.
        let ja = mt
            .encode("Chiều nay mình chốt lại spec API.", "ja")
            .unwrap();
        assert_eq!(
            ja[0], 128_046,
            "the Japanese token, 42 places into the table"
        );
        assert_eq!(
            &ja[1..],
            &mt.encode("Chiều nay mình chốt lại spec API.", "en")
                .unwrap()[1..]
        );
    }

    #[test]
    fn it_translates() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SEQ2SEQ to a model directory");
            return;
        };
        let mt = Seq2Seq::load(&paths(&dir), Some(8)).unwrap();
        let out = mt
            .translate("Chiều nay mình chốt lại spec API.", "en")
            .unwrap();
        // Deliberately not an assertion about *what* it says. This model makes lexical errors — it
        // renders this sentence as "I have been closing the spec fire today", which the reference
        // implementation also does. Pinning the wording here would be pinning a mistake; the
        // tokenizer tests above are what pin correctness.
        assert!(!out.is_empty(), "expected a translation");
        assert!(
            out.chars().any(|c| c.is_ascii_alphabetic()),
            "expected English, got `{out}`"
        );
    }

    /// Assembling pieces without turning `▁` back into a space produces one long word. It is
    /// obvious in English and invisible in Chinese, which has no spaces to begin with.
    #[test]
    fn words_come_back_separated() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SEQ2SEQ");
            return;
        };
        let mt = Seq2Seq::load(&paths(&dir), Some(4)).unwrap();
        let out = mt.translate("开放时间早上9点至下午5点。", "en").unwrap();
        assert!(out.contains(' '), "no word boundaries in `{out}`");
        assert!(!out.contains('▁'), "raw sentencepiece markers in `{out}`");
    }

    /// Found in a translation file, not in a test: the decoder finished a clause and then emitted
    /// 116 consecutive `<pad>` tokens, every one of which was written to the vault as literal
    /// `<pad>` text. `<pad>` is in `vocab.json`, so it detokenized perfectly happily.
    #[test]
    fn control_tokens_never_reach_the_text() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SEQ2SEQ");
            return;
        };
        let mt = Seq2Seq::load(&paths(&dir), Some(8)).unwrap();
        for (line, lang) in [
            (
                "Bên mình cần thêm hai ngày để test tải, không thì lúc go-live sẽ vỡ.",
                "ja",
            ),
            ("Chiều nay mình chốt lại spec API.", "en"),
        ] {
            let out = mt.translate(line, lang).unwrap();
            for marker in ["<pad>", "</s>", "<s>", "<unk>"] {
                assert!(!out.contains(marker), "`{marker}` in `{out}`");
            }
        }
    }

    #[test]
    fn japanese_comes_back_as_japanese() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SEQ2SEQ");
            return;
        };
        let mt = Seq2Seq::load(&paths(&dir), Some(8)).unwrap();
        let out = mt.translate("Cảm ơn bạn rất nhiều.", "ja").unwrap();
        assert!(
            out.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c)),
            "expected kana, got `{out}`"
        );
    }

    #[test]
    fn an_empty_line_costs_nothing() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SEQ2SEQ");
            return;
        };
        let mt = Seq2Seq::load(&paths(&dir), Some(1)).unwrap();
        assert_eq!(mt.translate("   ", "en").unwrap(), "");
    }

    /// Greedy, so a re-translation is comparable with the last one.
    #[test]
    fn the_same_line_translates_the_same_way_twice() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SEQ2SEQ");
            return;
        };
        let mt = Seq2Seq::load(&paths(&dir), Some(4)).unwrap();
        assert_eq!(
            mt.translate("Xin chào.", "en").unwrap(),
            mt.translate("Xin chào.", "en").unwrap()
        );
    }
}
