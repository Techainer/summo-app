//! Zipformer transducer decoding via sherpa-onnx.
//!
//! This is the runtime behind the Vietnamese models: an offline RNN-T transducer exported to INT8
//! ONNX. "Offline" here means non-causal, not slow — the prototype measured a real-time factor of
//! 0.017, which is the headroom that makes [`crate::PseudoSession`]'s re-decode loop affordable.
//!
//! Behind the `sherpa` feature, because it links a native library.

use std::path::Path;

use sherpa_rs::transducer::{TransducerConfig, TransducerRecognizer};
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
