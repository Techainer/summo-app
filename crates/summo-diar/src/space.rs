//! Which model's coordinate system a vector belongs to.
//!
//! An embedding is only meaningful next to other embeddings from the *same* model. Cosine
//! similarity between a CAM++ vector and an ERes2NetV2 vector is a well-defined number that means
//! nothing at all — the two models were trained separately and place voices in unrelated spaces.
//!
//! The trap is that this failure is invisible. CAM++ and ERes2NetV2 both emit **192 dimensions**, so
//! a dimension check passes and the arithmetic runs happily, returning similarities scattered around
//! zero. Every voice looks like a stranger, so Summo would quietly stop recognising people it had
//! already been taught — and worse, occasionally return a high score by chance and put the wrong
//! name on a sentence somebody said.
//!
//! So a stored vector carries the identity of the model that produced it, not just its shape.
//! Comparison across a mismatch is refused rather than approximated, and the fix is to re-embed from
//! the audio if it is still there, or to drop the vectors and start learning voices again if it is
//! not. Losing recognition is recoverable. Silently wrong attribution is not.

use serde::{Deserialize, Serialize};

/// The coordinate system a set of embeddings lives in.
///
/// Two vectors are comparable only when their spaces are [`EmbeddingSpace::compatible_with`] each
/// other, which means the same model *and* the same dimension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingSpace {
    /// Registry id of the model that produced the vectors, e.g. `campplus-sv`.
    pub model: String,
    /// Model revision. A retrained checkpoint under a reused id is still a different space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Vector length. Checked too, so a corrupt record is caught before it reaches the arithmetic.
    pub dims: usize,
}

impl EmbeddingSpace {
    #[must_use]
    pub fn new(model: impl Into<String>, dims: usize) -> Self {
        Self {
            model: model.into(),
            revision: None,
            dims,
        }
    }

    #[must_use]
    pub fn with_revision(mut self, revision: impl Into<String>) -> Self {
        self.revision = Some(revision.into());
        self
    }

    /// Whether vectors from `self` may be compared with vectors from `other`.
    ///
    /// A missing revision on either side is treated as compatible: vaults written before revisions
    /// were recorded should not all invalidate themselves on upgrade. An explicit disagreement is
    /// not compatible.
    #[must_use]
    pub fn compatible_with(&self, other: &Self) -> bool {
        if self.model != other.model || self.dims != other.dims {
            return false;
        }
        match (&self.revision, &other.revision) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }

    /// Explain a mismatch in terms a user can act on.
    #[must_use]
    pub fn describe_mismatch(&self, other: &Self) -> String {
        if self.model != other.model {
            return format!(
                "voices were learned with {} and this run uses {}; the two models do not share a \
                 coordinate system, so the stored voices cannot be compared",
                self.model, other.model
            );
        }
        if self.dims != other.dims {
            return format!(
                "voices were learned as {}-dimensional vectors and this run produces {}",
                self.dims, other.dims
            );
        }
        format!(
            "voices were learned with {} revision {} and this run uses revision {}",
            self.model,
            self.revision.as_deref().unwrap_or("unknown"),
            other.revision.as_deref().unwrap_or("unknown"),
        )
    }
}

impl std::fmt::Display for EmbeddingSpace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.model)?;
        if let Some(revision) = &self.revision {
            write!(f, "@{revision}")?;
        }
        write!(f, "/{}d", self.dims)
    }
}

/// What to do when stored vectors do not match the running model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Migration {
    /// Spaces agree; carry on.
    None,
    /// Audio is still on disk, so the vectors can be regenerated exactly.
    Reembed,
    /// Audio has been pruned. The vectors are unusable and recognition restarts from the names.
    Discard,
}

/// Decide what a space mismatch requires.
///
/// Names, transcripts and the fact that a person exists all survive either outcome — only the
/// vectors are model-specific. That is why this is a recoverable event rather than data loss.
#[must_use]
pub fn plan(
    stored: Option<&EmbeddingSpace>,
    running: &EmbeddingSpace,
    audio_kept: bool,
) -> Migration {
    match stored {
        // A book that has never held a vector adopts whatever model runs first.
        None => Migration::None,
        Some(stored) if stored.compatible_with(running) => Migration::None,
        Some(_) if audio_kept => Migration::Reembed,
        Some(_) => Migration::Discard,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn campplus() -> EmbeddingSpace {
        EmbeddingSpace::new("campplus-sv", 192)
    }

    fn eres2netv2() -> EmbeddingSpace {
        EmbeddingSpace::new("eres2netv2-sv", 192)
    }

    #[test]
    fn the_same_space_is_compatible() {
        assert!(campplus().compatible_with(&campplus()));
    }

    /// The whole reason this module exists: same dimension, different model.
    #[test]
    fn same_dimension_different_model_is_not_compatible() {
        assert_eq!(campplus().dims, eres2netv2().dims);
        assert!(!campplus().compatible_with(&eres2netv2()));
    }

    #[test]
    fn a_different_dimension_is_not_compatible() {
        assert!(!campplus().compatible_with(&EmbeddingSpace::new("campplus-sv", 256)));
    }

    #[test]
    fn a_retrained_checkpoint_is_a_different_space() {
        let v1 = campplus().with_revision("2024-03");
        let v2 = campplus().with_revision("2026-01");
        assert!(!v1.compatible_with(&v2));
        assert!(v1.compatible_with(&v1));
    }

    /// Vaults written before revisions were recorded must keep working.
    #[test]
    fn an_unknown_revision_does_not_invalidate_a_book() {
        assert!(campplus().compatible_with(&campplus().with_revision("2026-01")));
        assert!(
            campplus()
                .with_revision("2026-01")
                .compatible_with(&campplus())
        );
    }

    #[test]
    fn a_first_run_adopts_the_running_model() {
        assert_eq!(plan(None, &campplus(), false), Migration::None);
    }

    #[test]
    fn a_matching_space_needs_no_migration() {
        assert_eq!(plan(Some(&campplus()), &campplus(), false), Migration::None);
    }

    #[test]
    fn a_mismatch_reembeds_when_the_audio_survives() {
        assert_eq!(
            plan(Some(&campplus()), &eres2netv2(), true),
            Migration::Reembed
        );
    }

    #[test]
    fn a_mismatch_discards_when_the_audio_is_gone() {
        assert_eq!(
            plan(Some(&campplus()), &eres2netv2(), false),
            Migration::Discard
        );
    }

    #[test]
    fn a_mismatch_explains_itself_by_model_first() {
        let message = campplus().describe_mismatch(&eres2netv2());
        assert!(message.contains("campplus-sv"), "{message}");
        assert!(message.contains("eres2netv2-sv"), "{message}");
    }

    #[test]
    fn a_dimension_mismatch_says_so() {
        let message = campplus().describe_mismatch(&EmbeddingSpace::new("campplus-sv", 256));
        assert!(message.contains("192"), "{message}");
        assert!(message.contains("256"), "{message}");
    }

    #[test]
    fn display_is_readable() {
        assert_eq!(campplus().to_string(), "campplus-sv/192d");
        assert_eq!(
            campplus().with_revision("2026-01").to_string(),
            "campplus-sv@2026-01/192d"
        );
    }

    #[test]
    fn a_space_round_trips_through_json() {
        let space = campplus().with_revision("2026-01");
        let json = serde_json::to_string(&space).expect("serialise");
        let back: EmbeddingSpace = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(space, back);
    }
}
