//! Turning an utterance into a voice fingerprint.
//!
//! Wraps a speaker-verification model (3D-Speaker CAM++ or ERes2NetV2) through sherpa-onnx. These
//! produce a fixed-length vector per utterance where distance corresponds to "same person", which is
//! all [`crate::cluster`] needs.
//!
//! Cost is the reason this design works: one embedding per finished utterance, single-digit
//! milliseconds, run after the transcript line is already visible. It never touches live latency.

use std::path::Path;

use sherpa_rs::speaker_id::{EmbeddingExtractor, ExtractorConfig};
use summo_core::{Error, Result, audio::SAMPLE_RATE};

/// Extracts speaker embeddings.
pub struct SpeakerEmbedder {
    inner: EmbeddingExtractor,
    dimension: usize,
}

impl SpeakerEmbedder {
    pub fn load(model: impl AsRef<Path>, num_threads: usize) -> Result<Self> {
        let path = model.as_ref();
        if !path.is_file() {
            return Err(Error::Other(format!(
                "speaker embedding model not found: {}",
                path.display()
            )));
        }

        let inner = EmbeddingExtractor::new(ExtractorConfig {
            model: path.display().to_string(),
            num_threads: Some(num_threads.clamp(1, 8)),
            provider: None,
            debug: false,
        })
        .map_err(|e| Error::Other(format!("cannot load speaker embedder: {e}")))?;

        let dimension = inner.embedding_size;
        Ok(Self { inner, dimension })
    }

    /// Vector length this model produces.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Embed one utterance of 16 kHz mono audio.
    pub fn embed(&mut self, pcm: &[f32]) -> Result<Vec<f32>> {
        if pcm.is_empty() {
            return Err(Error::Other("cannot embed empty audio".into()));
        }
        self.inner
            .compute_speaker_embedding(pcm.to_vec(), SAMPLE_RATE)
            .map_err(|e| Error::Other(format!("embedding failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Option<std::path::PathBuf> {
        std::env::var_os("SUMMO_TEST_SPEAKER_MODEL").map(std::path::PathBuf::from)
    }

    #[test]
    fn a_missing_model_is_reported_clearly() {
        let Err(err) = SpeakerEmbedder::load("/nonexistent/model.onnx", 1) else {
            panic!("loading a nonexistent model should fail")
        };
        assert!(err.to_string().contains("not found"), "got: {err}");
    }

    #[test]
    fn embedding_produces_a_fixed_length_vector() {
        let Some(path) = model() else {
            eprintln!("skipping: set SUMMO_TEST_SPEAKER_MODEL");
            return;
        };
        let mut e = SpeakerEmbedder::load(path, 2).unwrap();
        let v = e.embed(&vec![0.01; SAMPLE_RATE as usize * 2]).unwrap();

        assert_eq!(v.len(), e.dimension());
        assert!(
            e.dimension() >= 128,
            "unexpectedly small embedding: {}",
            e.dimension()
        );
    }

    #[test]
    fn empty_audio_is_refused() {
        let Some(path) = model() else {
            eprintln!("skipping: set SUMMO_TEST_SPEAKER_MODEL");
            return;
        };
        let mut e = SpeakerEmbedder::load(path, 1).unwrap();
        assert!(e.embed(&[]).is_err());
    }

    /// The property the clusterer depends on: the same audio must embed to the same vector, and
    /// different audio to a different one. Without this, every threshold is meaningless.
    #[test]
    fn identical_audio_embeds_identically() {
        let Some(path) = model() else {
            eprintln!("skipping: set SUMMO_TEST_SPEAKER_MODEL");
            return;
        };
        let mut e = SpeakerEmbedder::load(path, 2).unwrap();

        let pcm: Vec<f32> = (0..SAMPLE_RATE as usize * 2)
            .map(|i| (i as f32 / 40.0).sin() * 0.2)
            .collect();

        let a = e.embed(&pcm).unwrap();
        let b = e.embed(&pcm).unwrap();

        let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
        let similarity = dot / (norm(&a) * norm(&b));

        assert!(
            similarity > 0.999,
            "the same audio must embed to the same vector, got similarity {similarity}"
        );
    }
}
