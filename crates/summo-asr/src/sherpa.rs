//! Zipformer transducer decoding via sherpa-onnx.
//!
//! This is the runtime behind the Vietnamese models: an offline RNN-T transducer exported to INT8
//! ONNX. "Offline" here means non-causal, not slow — the prototype measured a real-time factor of
//! 0.017, which is the headroom that makes [`crate::PseudoSession`]'s re-decode loop affordable.
//!
//! Behind the `sherpa` feature, because it links a native library.

use std::path::Path;

use sherpa_rs::{
    sense_voice::{SenseVoiceConfig, SenseVoiceRecognizer},
    transducer::{TransducerConfig, TransducerRecognizer},
    whisper::{WhisperConfig, WhisperRecognizer},
};
use summo_core::{Error, Result, audio::SAMPLE_RATE};

use crate::decoder::{Decoder, Transcript};

/// Files a transducer model needs.
#[derive(Debug, Clone)]
pub struct TransducerPaths {
    pub encoder: String,
    pub decoder: String,
    pub joiner: String,
    pub tokens: String,
}

impl TransducerPaths {
    /// Resolve the four files from a model directory by matching name fragments.
    ///
    /// Checkpoints are published with epoch and averaging in the file name
    /// (`encoder-epoch-35-avg-6.int8.onnx`), so an exact match is not possible and the manifest's
    /// `params` are the real source of truth. This is the convenience path for a directory a user
    /// unpacked by hand.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;

        let (mut encoder, mut decoder, mut joiner, mut tokens) = (None, None, None, None);
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let slot = if name.contains("encoder") {
                &mut encoder
            } else if name.contains("decoder") {
                &mut decoder
            } else if name.contains("joiner") {
                &mut joiner
            } else if name == "tokens.txt" {
                &mut tokens
            } else {
                continue;
            };
            // Prefer the quantized export when both are present: it is what the profile was
            // measured against, and the accuracy difference is inside the noise.
            let is_int8 = name.contains("int8");
            if slot.is_none() || is_int8 {
                *slot = Some(path.display().to_string());
            }
        }

        let missing = |what: &str| Error::Asr(format!("{}: no {what} file found", dir.display()));
        Ok(Self {
            encoder: encoder.ok_or_else(|| missing("encoder"))?,
            decoder: decoder.ok_or_else(|| missing("decoder"))?,
            joiner: joiner.ok_or_else(|| missing("joiner"))?,
            tokens: tokens.ok_or_else(|| missing("tokens.txt"))?,
        })
    }
}

/// An offline Zipformer transducer.
pub struct ZipformerDecoder {
    inner: TransducerRecognizer,
    name: String,
}

impl ZipformerDecoder {
    pub fn load(
        paths: &TransducerPaths,
        num_threads: usize,
        name: impl Into<String>,
    ) -> Result<Self> {
        for (label, path) in [
            ("encoder", &paths.encoder),
            ("decoder", &paths.decoder),
            ("joiner", &paths.joiner),
            ("tokens", &paths.tokens),
        ] {
            if !Path::new(path).is_file() {
                return Err(Error::Asr(format!("{label} file not found: {path}")));
            }
        }

        let config = TransducerConfig {
            encoder: paths.encoder.clone(),
            decoder: paths.decoder.clone(),
            joiner: paths.joiner.clone(),
            tokens: paths.tokens.clone(),
            num_threads: i32::try_from(num_threads.clamp(1, 16)).unwrap_or(1),
            sample_rate: i32::try_from(SAMPLE_RATE).unwrap_or(16_000),
            feature_dim: 80,
            decoding_method: "greedy_search".into(),
            ..TransducerConfig::default()
        };

        let inner = TransducerRecognizer::new(config)
            .map_err(|e| Error::Asr(format!("cannot load transducer: {e}")))?;

        Ok(Self {
            inner,
            name: name.into(),
        })
    }

    /// Load from a directory containing the four model files.
    pub fn from_dir(dir: impl AsRef<Path>, num_threads: usize) -> Result<Self> {
        let dir = dir.as_ref();
        let name = dir.file_name().map_or_else(
            || "zipformer".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self::load(&TransducerPaths::from_dir(dir)?, num_threads, name)
    }
}

impl Decoder for ZipformerDecoder {
    fn decode(&mut self, pcm: &[f32]) -> Result<Transcript> {
        // A transducer given almost nothing returns an empty string; skipping the call avoids the
        // fixed per-decode overhead during the arming phase of an utterance.
        if pcm.len() < SAMPLE_RATE as usize / 20 {
            return Ok(Transcript::default());
        }
        let text = self.inner.transcribe(SAMPLE_RATE, pcm);
        Ok(Transcript {
            text: text.trim().to_string(),
            // Transducers do not expose a no-speech probability. They also do not hallucinate
            // subtitle boilerplate over silence, which is what that signal is for.
            ..Transcript::default()
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directory holding a transducer checkpoint. Tests are skipped without it, so the suite runs
    /// on a machine that has not downloaded any models.
    fn model_dir() -> Option<std::path::PathBuf> {
        std::env::var_os("SUMMO_TEST_TRANSDUCER").map(std::path::PathBuf::from)
    }

    #[test]
    fn missing_files_fail_before_the_native_library_is_called() {
        let paths = TransducerPaths {
            encoder: "/nonexistent/encoder.onnx".into(),
            decoder: "/nonexistent/decoder.onnx".into(),
            joiner: "/nonexistent/joiner.onnx".into(),
            tokens: "/nonexistent/tokens.txt".into(),
        };
        let Err(err) = ZipformerDecoder::load(&paths, 1, "test") else {
            panic!("loading nonexistent files should fail")
        };
        assert!(err.to_string().contains("encoder"), "got: {err}");
    }

    #[test]
    fn an_empty_directory_reports_what_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let err = TransducerPaths::from_dir(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("encoder"), "got: {err}");
    }

    #[test]
    fn quantized_exports_are_preferred_over_float() {
        let tmp = tempfile::tempdir().unwrap();
        for name in [
            "encoder-epoch-35-avg-6.onnx",
            "encoder-epoch-35-avg-6.int8.onnx",
            "decoder-epoch-35-avg-6.int8.onnx",
            "joiner-epoch-35-avg-6.int8.onnx",
            "tokens.txt",
        ] {
            std::fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let paths = TransducerPaths::from_dir(tmp.path()).unwrap();
        assert!(
            paths.encoder.contains("int8"),
            "should prefer the quantized encoder, got {}",
            paths.encoder
        );
    }

    #[test]
    fn very_short_audio_is_not_sent_to_the_model() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_TRANSDUCER to a model directory");
            return;
        };
        let mut d = ZipformerDecoder::from_dir(dir, 2).unwrap();
        assert!(d.decode(&vec![0.0; 100]).unwrap().is_empty());
    }

    #[test]
    fn silence_decodes_to_nothing() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_TRANSDUCER to a model directory");
            return;
        };
        let mut d = ZipformerDecoder::from_dir(dir, 2).unwrap();
        let out = d.decode(&vec![0.0; SAMPLE_RATE as usize]).unwrap();
        assert!(
            out.is_empty(),
            "a transducer should stay quiet on silence, got `{}`",
            out.text
        );
    }
}

// ---------------------------------------------------------------------------------------- whisper

/// Files a Whisper export needs.
#[derive(Debug, Clone)]
pub struct WhisperPaths {
    pub encoder: String,
    pub decoder: String,
    pub tokens: String,
}

impl WhisperPaths {
    /// Resolve `<name>-encoder.onnx`, `<name>-decoder.onnx` and `<name>-tokens.txt` from a
    /// directory, preferring the quantized export.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;

        let (mut encoder, mut decoder, mut tokens) = (None, None, None);
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !name.ends_with(".onnx") && !name.ends_with(".txt") {
                continue;
            }
            let slot = if name.contains("encoder") {
                &mut encoder
            } else if name.contains("decoder") {
                &mut decoder
            } else if name.contains("tokens") {
                &mut tokens
            } else {
                continue;
            };
            let is_int8 = name.contains("int8");
            if slot.is_none() || is_int8 {
                *slot = Some(path.display().to_string());
            }
        }

        let missing = |what: &str| Error::Asr(format!("{}: no {what} file found", dir.display()));
        Ok(Self {
            encoder: encoder.ok_or_else(|| missing("encoder"))?,
            decoder: decoder.ok_or_else(|| missing("decoder"))?,
            tokens: tokens.ok_or_else(|| missing("tokens"))?,
        })
    }
}

/// Whisper, for the languages a Vietnamese-only transducer cannot cover.
///
/// Whisper's strength is breadth — 99 languages and code-switching within a sentence — and its
/// weakness is that it invents text. Trained on subtitles, given silence it returns what subtitles
/// say over silence. [`crate::HallucinationFilter`] exists for this model family, and
/// [`Decoder::supports_partials`] returns `false` because a half-spoken utterance reliably produces
/// a confident guess at an ending that was never said.
pub struct WhisperDecoder {
    inner: WhisperRecognizer,
    name: String,
    language: String,
}

impl WhisperDecoder {
    /// `language` is an ISO code such as `en` or `vi`; `None` asks the model to detect it.
    ///
    /// Detection costs a little accuracy and, in a meeting that switches between two languages, can
    /// flip mid-recording. Naming the language is better whenever it is known.
    pub fn load(
        paths: &WhisperPaths,
        language: Option<&str>,
        num_threads: usize,
        name: impl Into<String>,
    ) -> Result<Self> {
        for (label, path) in [
            ("encoder", &paths.encoder),
            ("decoder", &paths.decoder),
            ("tokens", &paths.tokens),
        ] {
            if !Path::new(path).is_file() {
                return Err(Error::Asr(format!("{label} file not found: {path}")));
            }
        }

        let language = language.unwrap_or("").to_string();
        let config = WhisperConfig {
            encoder: paths.encoder.clone(),
            decoder: paths.decoder.clone(),
            tokens: paths.tokens.clone(),
            language: language.clone(),
            num_threads: Some(i32::try_from(num_threads.clamp(1, 16)).unwrap_or(1)),
            ..WhisperConfig::default()
        };

        let inner = WhisperRecognizer::new(config)
            .map_err(|e| Error::Asr(format!("cannot load whisper: {e}")))?;

        Ok(Self {
            inner,
            name: name.into(),
            language,
        })
    }

    pub fn from_dir(
        dir: impl AsRef<Path>,
        language: Option<&str>,
        num_threads: usize,
    ) -> Result<Self> {
        let dir = dir.as_ref();
        let name = dir.file_name().map_or_else(
            || "whisper".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self::load(&WhisperPaths::from_dir(dir)?, language, num_threads, name)
    }

    /// The configured language, or `"auto"` when detection is in use.
    #[must_use]
    pub fn language(&self) -> &str {
        if self.language.is_empty() {
            "auto"
        } else {
            &self.language
        }
    }
}

impl Decoder for WhisperDecoder {
    fn decode(&mut self, pcm: &[f32]) -> Result<Transcript> {
        // Whisper pads everything to a 30 second window internally, so a very short clip costs the
        // same as a long one and reliably returns invented text. Not worth the call.
        if pcm.len() < SAMPLE_RATE as usize / 4 {
            return Ok(Transcript::default());
        }
        let result = self.inner.transcribe(SAMPLE_RATE, pcm);
        Ok(Transcript {
            text: result.text.trim().to_string(),
            language: spoken_language(&result.lang),
            ..Transcript::default()
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_partials(&self) -> bool {
        // See the type docs: a partial utterance produces a confident guess at an ending nobody
        // said, which looks far worse to a user than text arriving a beat later.
        false
    }
}

#[cfg(test)]
mod whisper_tests {
    use super::*;

    fn model_dir() -> Option<std::path::PathBuf> {
        std::env::var_os("SUMMO_TEST_WHISPER").map(std::path::PathBuf::from)
    }

    #[test]
    fn missing_files_are_reported_before_loading() {
        let paths = WhisperPaths {
            encoder: "/nonexistent/e.onnx".into(),
            decoder: "/nonexistent/d.onnx".into(),
            tokens: "/nonexistent/t.txt".into(),
        };
        let Err(err) = WhisperDecoder::load(&paths, Some("en"), 1, "test") else {
            panic!("loading nonexistent files should fail")
        };
        assert!(err.to_string().contains("encoder"), "got: {err}");
    }

    #[test]
    fn quantized_exports_win() {
        let tmp = tempfile::tempdir().unwrap();
        for name in [
            "tiny-encoder.onnx",
            "tiny-encoder.int8.onnx",
            "tiny-decoder.int8.onnx",
            "tiny-tokens.txt",
        ] {
            std::fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let paths = WhisperPaths::from_dir(tmp.path()).unwrap();
        assert!(paths.encoder.contains("int8"), "got {}", paths.encoder);
    }

    #[test]
    fn whisper_declines_partial_decoding() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_WHISPER");
            return;
        };
        let d = WhisperDecoder::from_dir(dir, Some("en"), 2).unwrap();
        assert!(
            !d.supports_partials(),
            "a batch model that hallucinates endings must not drive live partials"
        );
    }

    #[test]
    fn english_speech_is_transcribed() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_WHISPER");
            return;
        };
        let Some(wav) = std::env::var_os("SUMMO_TEST_WAV_EN").map(std::path::PathBuf::from) else {
            eprintln!("skipping: set SUMMO_TEST_WAV_EN");
            return;
        };

        let mut reader = hound::WavReader::open(&wav).unwrap();
        let scale = 1.0 / f32::from(i16::MAX);
        let pcm: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) * scale)
            .collect();

        let mut d = WhisperDecoder::from_dir(dir, Some("en"), 4).unwrap();
        let out = d.decode(&pcm).unwrap();

        assert!(!out.is_empty(), "expected English text, got nothing");
        assert!(
            out.text.split_whitespace().count() > 3,
            "expected a sentence, got `{}`",
            out.text
        );
    }

    #[test]
    fn language_reports_auto_when_unset() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_WHISPER");
            return;
        };
        let d = WhisperDecoder::from_dir(dir, None, 1).unwrap();
        assert_eq!(d.language(), "auto");
    }
}

// ------------------------------------------------------------------------------------ sensevoice

/// Files a SenseVoice export needs.
///
/// One `.onnx` and one `tokens.txt`, which is the whole model — SenseVoice is a single
/// non-autoregressive encoder rather than the encoder/decoder pair every other runtime here needs.
#[derive(Debug, Clone)]
pub struct SenseVoicePaths {
    pub model: String,
    pub tokens: String,
}

impl SenseVoicePaths {
    /// Resolve the two files from a directory, preferring the quantized export.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;

        let (mut model, mut tokens) = (None, None);
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let slot = if name.ends_with(".onnx") {
                &mut model
            } else if name.contains("tokens") && name.ends_with(".txt") {
                &mut tokens
            } else {
                continue;
            };
            let is_int8 = name.contains("int8");
            if slot.is_none() || is_int8 {
                *slot = Some(path.display().to_string());
            }
        }

        let missing = |what: &str| Error::Asr(format!("{}: no {what} file found", dir.display()));
        Ok(Self {
            model: model.ok_or_else(|| missing("model .onnx"))?,
            tokens: tokens.ok_or_else(|| missing("tokens"))?,
        })
    }
}

/// SenseVoice, for Chinese, Japanese, Korean, Cantonese and English.
///
/// The reason this runtime exists alongside Whisper, which nominally covers the same languages:
///
/// * **It does not invent text.** SenseVoice is non-autoregressive — one encoder pass, CTC-style
///   output — so there is no decoder to run away into a plausible ending nobody said. That is what
///   makes [`Decoder::supports_partials`] `true` here and `false` for Whisper, and it is the
///   difference between text appearing while somebody is still talking and text appearing after
///   they stop.
/// * **It is much faster.** No autoregression means the cost is one pass over the audio rather than
///   one pass per generated token, which is what [`crate::PseudoSession`]'s re-decode loop spends
///   its headroom on.
///
/// It cannot do Vietnamese, which is not a gap: the Vietnamese transducer beats every multilingual
/// model on that language by a wide margin, and picking a model per language is the point of having
/// more than one runtime.
pub struct SenseVoiceDecoder {
    inner: SenseVoiceRecognizer,
    name: String,
    language: String,
}

impl SenseVoiceDecoder {
    /// `language` is `zh`, `en`, `ja`, `ko` or `yue`; `None` lets the model detect it.
    ///
    /// Unlike Whisper's, SenseVoice's detection is a classifier head rather than generated tokens,
    /// so `auto` is cheap and reliable enough to be a reasonable default for a meeting that
    /// switches between languages.
    pub fn load(
        paths: &SenseVoicePaths,
        language: Option<&str>,
        num_threads: usize,
        name: impl Into<String>,
    ) -> Result<Self> {
        for (label, path) in [("model", &paths.model), ("tokens", &paths.tokens)] {
            if !Path::new(path).is_file() {
                return Err(Error::Asr(format!("{label} file not found: {path}")));
            }
        }

        // `auto` rather than an empty string: SenseVoice's config takes the word, and an empty one
        // selects no language head at all.
        let language = language
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .unwrap_or("auto")
            .to_string();

        let config = SenseVoiceConfig {
            model: paths.model.clone(),
            tokens: paths.tokens.clone(),
            language: language.clone(),
            // Inverse text normalisation: "二零二六年" becomes "2026年". Meeting notes are read for
            // the numbers in them, and a date spelled out in characters is one a reader has to
            // decode.
            use_itn: true,
            num_threads: Some(i32::try_from(num_threads.clamp(1, 16)).unwrap_or(1)),
            ..SenseVoiceConfig::default()
        };

        let inner = SenseVoiceRecognizer::new(config)
            .map_err(|e| Error::Asr(format!("cannot load sensevoice: {e}")))?;

        Ok(Self {
            inner,
            name: name.into(),
            language,
        })
    }

    pub fn from_dir(
        dir: impl AsRef<Path>,
        language: Option<&str>,
        num_threads: usize,
    ) -> Result<Self> {
        let dir = dir.as_ref();
        let name = dir.file_name().map_or_else(
            || "sensevoice".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self::load(
            &SenseVoicePaths::from_dir(dir)?,
            language,
            num_threads,
            name,
        )
    }

    /// The configured language, or `"auto"` when detection is in use.
    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }
}

/// Strip SenseVoice's rich-transcription markers.
///
/// The model emits language, emotion and audio-event labels inline —
/// `<|ja|><|NEUTRAL|><|Speech|><|withitn|>こんにちは` — and which of them survive into the text
/// depends on the sherpa-onnx build. They are not speech: left in, they land in the Markdown file,
/// the summary the model is asked to write, and the subtitle burnt into a video.
///
/// Written as a scan rather than a regex because an unterminated `<|` must leave the rest of the
/// line alone rather than eat it.
#[must_use]
pub fn strip_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("<|") {
        let (before, after) = rest.split_at(start);
        out.push_str(before);
        match after.find("|>") {
            Some(end) => rest = &after[end + 2..],
            // No closing marker: this is ordinary text that happens to contain `<|`.
            None => {
                out.push_str(after);
                return out.trim().to_string();
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

impl Decoder for SenseVoiceDecoder {
    fn decode(&mut self, pcm: &[f32]) -> Result<Transcript> {
        // Below about a syllable there is nothing to recognise, and the fixed per-decode cost is
        // paid on every pass of the re-decode loop.
        if pcm.len() < SAMPLE_RATE as usize / 20 {
            return Ok(Transcript::default());
        }
        let result = self.inner.transcribe(SAMPLE_RATE, pcm);
        Ok(Transcript {
            text: strip_tags(&result.text),
            language: spoken_language(&result.lang),
            // Non-autoregressive: there is no no-speech probability, and no hallucinated ending for
            // one to guard against.
            ..Transcript::default()
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod sensevoice_tests {
    use super::*;

    fn model_dir() -> Option<std::path::PathBuf> {
        std::env::var_os("SUMMO_TEST_SENSEVOICE").map(std::path::PathBuf::from)
    }

    #[test]
    fn missing_files_are_reported_before_loading() {
        let paths = SenseVoicePaths {
            model: "/nonexistent/model.onnx".into(),
            tokens: "/nonexistent/tokens.txt".into(),
        };
        let Err(err) = SenseVoiceDecoder::load(&paths, Some("ja"), 1, "test") else {
            panic!("loading nonexistent files should fail")
        };
        assert!(err.to_string().contains("model"), "got: {err}");
    }

    #[test]
    fn quantized_exports_win() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["model.onnx", "model.int8.onnx", "tokens.txt"] {
            std::fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let paths = SenseVoicePaths::from_dir(tmp.path()).unwrap();
        assert!(paths.model.contains("int8"), "got {}", paths.model);
    }

    #[test]
    fn an_empty_directory_reports_what_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let err = SenseVoicePaths::from_dir(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("model"), "got: {err}");
    }

    /// The failure this prevents is silent and permanent: the tags are written to the vault, so a
    /// meeting recorded before anyone noticed keeps them forever.
    #[test]
    fn rich_transcription_markers_are_not_speech() {
        assert_eq!(
            strip_tags("<|ja|><|NEUTRAL|><|Speech|><|withitn|>こんにちは"),
            "こんにちは"
        );
        assert_eq!(strip_tags("<|zh|>今天开会"), "今天开会");
        assert_eq!(strip_tags("  plain text  "), "plain text");
    }

    /// A transcript is arbitrary human speech and may contain the characters the marker is made of.
    /// Eating the rest of the line on an unterminated `<|` would lose real words.
    #[test]
    fn an_unterminated_marker_leaves_the_text_alone() {
        assert_eq!(
            strip_tags("the operator <| means pipe"),
            "the operator <| means pipe"
        );
    }

    #[test]
    fn markers_between_words_are_removed_without_joining_them() {
        assert_eq!(strip_tags("hello <|EMO|> world"), "hello  world");
    }

    /// The reason this runtime exists next to Whisper: it can drive live text, and Whisper cannot.
    #[test]
    fn sensevoice_can_drive_partials() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SENSEVOICE to a model directory");
            return;
        };
        let d = SenseVoiceDecoder::from_dir(dir, Some("ja"), 2).unwrap();
        assert!(d.supports_partials());
    }

    #[test]
    fn language_reports_auto_when_unset() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SENSEVOICE");
            return;
        };
        let d = SenseVoiceDecoder::from_dir(dir, None, 1).unwrap();
        assert_eq!(d.language(), "auto");
    }

    #[test]
    fn silence_decodes_to_nothing() {
        let Some(dir) = model_dir() else {
            eprintln!("skipping: set SUMMO_TEST_SENSEVOICE");
            return;
        };
        let mut d = SenseVoiceDecoder::from_dir(dir, Some("zh"), 2).unwrap();
        let out = d.decode(&vec![0.0; SAMPLE_RATE as usize]).unwrap();
        assert!(out.is_empty(), "expected silence, got `{}`", out.text);
    }
}

/// Normalise what sherpa reports as the detected language.
///
/// Whisper hands back its own token — `<|en|>` — and SenseVoice hands back a bare code or nothing
/// at all. Neither shape is one a caller should have to know about, and the empty string is the
/// runtime saying "I was told the language, I did not detect one", which is `None` rather than a
/// language whose tag is empty.
///
/// Lower-cased, because everything downstream compares against a manifest's own spelling and `EN`
/// from one runtime must not read as a language no model covers.
fn spoken_language(reported: &str) -> Option<String> {
    let trimmed = reported
        .trim()
        .trim_start_matches("<|")
        .trim_end_matches("|>")
        .trim();
    (!trimmed.is_empty()).then(|| trimmed.to_lowercase())
}

#[cfg(test)]
mod language_tests {
    use super::spoken_language;

    /// Whisper answers in its own token vocabulary; nothing outside this file should have to know.
    #[test]
    fn a_whisper_token_becomes_a_plain_code() {
        assert_eq!(spoken_language("<|en|>").as_deref(), Some("en"));
        assert_eq!(spoken_language("vi").as_deref(), Some("vi"));
        assert_eq!(spoken_language(" <|JA|> ").as_deref(), Some("ja"));
    }

    /// A model told which language to use reports nothing, and "nothing" is not a language.
    /// Routing on an empty tag would send every utterance to whichever model claims `""`, which is
    /// none of them — so every utterance would lose its refinement.
    #[test]
    fn silence_from_the_runtime_is_not_a_language() {
        assert_eq!(spoken_language(""), None);
        assert_eq!(spoken_language("   "), None);
        assert_eq!(spoken_language("<||>"), None);
    }
}
