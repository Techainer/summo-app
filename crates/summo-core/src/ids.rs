//! Newtype identifiers.
//!
//! These exist to stop the classic bug where a meeting id and a model id are both `String` and get
//! swapped at a call site that still type-checks.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Time-sortable meeting identifier (UUIDv7), so the vault sorts chronologically on disk.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MeetingId(String);

impl MeetingId {
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MeetingId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<String> for MeetingId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for MeetingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Registry model identifier, e.g. `gipformer-65m`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    /// Model ids double as directory names, so they are restricted to a filesystem-safe subset.
    pub fn parse(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();
        if s.is_empty() || s.len() > 128 {
            return Err(format!("model id must be 1..=128 chars, got {}", s.len()));
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '_'))
        {
            return Err(format!(
                "model id `{s}` must match [a-z0-9._-]+ so it is safe as a path segment"
            ));
        }
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Diarization label. Starts as an auto-assigned `S1`/`S2`, becomes a real name once the user
/// renames it or an enrolled voice profile matches.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpeakerId(String);

impl SpeakerId {
    /// The local user, always attributed from the mic lane rather than by clustering.
    #[must_use]
    pub fn me() -> Self {
        Self("me".into())
    }

    /// Auto-assigned label for the `index`-th distinct voice found in a remote lane.
    #[must_use]
    pub fn auto(index: usize) -> Self {
        Self(format!("S{}", index + 1))
    }

    #[must_use]
    pub fn is_me(&self) -> bool {
        self.0 == "me"
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SpeakerId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for SpeakerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_ids_sort_chronologically() {
        let a = MeetingId::new();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = MeetingId::new();
        assert!(a < b, "uuidv7 must keep vault listings in time order");
    }

    #[test]
    fn model_id_rejects_path_traversal() {
        assert!(ModelId::parse("../../etc/passwd").is_err());
        assert!(
            ModelId::parse("Gipformer").is_err(),
            "uppercase would break case-insensitive fs"
        );
        assert!(ModelId::parse("").is_err());
        assert!(ModelId::parse("gipformer-65m").is_ok());
        assert!(ModelId::parse("whisper-large-v3-turbo.q5_k").is_ok());
    }

    #[test]
    fn auto_speaker_labels_are_one_based() {
        assert_eq!(SpeakerId::auto(0).as_str(), "S1");
        assert_eq!(SpeakerId::auto(2).as_str(), "S3");
        assert!(SpeakerId::me().is_me());
        assert!(!SpeakerId::auto(0).is_me());
    }
}
