//! Transcript units.
//!
//! A [`Segment`] is one VAD-delimited utterance. It is created the moment speech starts, updated
//! with partial text while speech continues, finalized on trailing silence, and possibly *revised*
//! later by a slower, more accurate model. The `source` field records which of those produced the
//! current text so the UI can render it differently.

use serde::{Deserialize, Serialize};

use crate::ids::SpeakerId;

/// Which capture track an utterance came from.
///
/// This is the cheapest and most reliable diarization signal we have: anything on [`Lane::Mic`] is
/// the local user by construction, no clustering required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    /// Local microphone — the person using Summo.
    Mic,
    /// System audio loopback — everyone else in the call.
    System,
}

impl Lane {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::System => "system",
        }
    }

    /// Wire tag prefixed to binary PCM frames so one socket can carry both tracks.
    #[must_use]
    pub fn tag(self) -> u8 {
        match self {
            Self::Mic => 0,
            Self::System => 1,
        }
    }

    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Mic),
            1 => Some(Self::System),
            _ => None,
        }
    }

    /// Speaker attribution that needs no model at all.
    #[must_use]
    pub fn default_speaker(self) -> Option<SpeakerId> {
        match self {
            Self::Mic => Some(SpeakerId::me()),
            Self::System => None,
        }
    }
}

/// Where a segment's current text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SegmentSource {
    /// Streaming model, still mid-utterance. Rendered dimmed.
    Partial,
    /// Committed by the primary model on trailing silence.
    Final,
    /// Replaced by the slower refine model after the fact.
    Revised,
    /// Edited by hand. Never overwritten by a model again.
    Manual,
}

impl SegmentSource {
    /// Whether an incoming update from `next` may overwrite text currently at `self`.
    ///
    /// The ordering matters: a late `Partial` from a re-decode must not clobber a `Final`, and
    /// nothing at all may clobber a `Manual` edit.
    #[must_use]
    pub fn accepts(self, next: Self) -> bool {
        match (self, next) {
            (Self::Manual, _) => false,
            (_, Self::Manual) => true,
            (Self::Partial, _) => true,
            (Self::Final, Self::Final | Self::Revised) => true,
            (Self::Final, Self::Partial) => false,
            (Self::Revised, Self::Revised) => true,
            (Self::Revised, _) => false,
        }
    }
}

/// Word-level timing, used for karaoke highlight and click-to-seek.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    /// Session-relative start, seconds.
    pub t0: f64,
    /// Session-relative end, seconds.
    pub t1: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conf: Option<f32>,
}

/// One VAD-delimited utterance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Monotonic per-session index. Stable across partial → final → revise.
    pub seq: u64,
    pub lane: Lane,
    pub text: String,
    /// Session-relative start, seconds.
    pub t0: f64,
    /// Session-relative end, seconds.
    pub t1: f64,
    pub source: SegmentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<SpeakerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conf: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
}

impl Segment {
    #[must_use]
    pub fn new(seq: u64, lane: Lane, text: impl Into<String>, t0: f64, t1: f64) -> Self {
        Self {
            seq,
            lane,
            text: text.into(),
            t0,
            t1,
            source: SegmentSource::Partial,
            speaker: lane.default_speaker(),
            conf: None,
            words: Vec::new(),
        }
    }

    #[must_use]
    pub fn duration(&self) -> f64 {
        (self.t1 - self.t0).max(0.0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Apply an update, honouring [`SegmentSource::accepts`]. Returns whether anything changed.
    pub fn merge(&mut self, incoming: &Segment) -> bool {
        if !self.source.accepts(incoming.source) {
            return false;
        }
        self.text.clone_from(&incoming.text);
        self.t1 = incoming.t1;
        self.source = incoming.source;
        if incoming.speaker.is_some() {
            self.speaker.clone_from(&incoming.speaker);
        }
        if incoming.conf.is_some() {
            self.conf = incoming.conf;
        }
        if !incoming.words.is_empty() {
            self.words.clone_from(&incoming.words);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(source: SegmentSource, text: &str) -> Segment {
        let mut s = Segment::new(1, Lane::System, text, 0.0, 1.0);
        s.source = source;
        s
    }

    #[test]
    fn mic_lane_is_attributed_without_a_model() {
        assert_eq!(
            Segment::new(0, Lane::Mic, "hi", 0.0, 1.0).speaker,
            Some(SpeakerId::me())
        );
        assert_eq!(Segment::new(0, Lane::System, "hi", 0.0, 1.0).speaker, None);
    }

    #[test]
    fn late_partial_never_clobbers_final() {
        let mut s = seg(SegmentSource::Final, "câu đã chốt");
        let stale = seg(SegmentSource::Partial, "câu đã ch");
        assert!(!s.merge(&stale));
        assert_eq!(s.text, "câu đã chốt");
    }

    #[test]
    fn refine_model_may_revise_a_final() {
        let mut s = seg(SegmentSource::Final, "toi nghi minh nen");
        let refined = seg(SegmentSource::Revised, "Tôi nghĩ mình nên");
        assert!(s.merge(&refined));
        assert_eq!(s.text, "Tôi nghĩ mình nên");
        assert_eq!(s.source, SegmentSource::Revised);
    }

    #[test]
    fn manual_edits_are_never_overwritten() {
        let mut s = seg(SegmentSource::Manual, "bản sửa tay");
        for src in [
            SegmentSource::Partial,
            SegmentSource::Final,
            SegmentSource::Revised,
        ] {
            assert!(!s.merge(&seg(src, "model output")));
        }
        assert_eq!(s.text, "bản sửa tay");
    }

    #[test]
    fn lane_tags_round_trip() {
        for lane in [Lane::Mic, Lane::System] {
            assert_eq!(Lane::from_tag(lane.tag()), Some(lane));
        }
        assert_eq!(Lane::from_tag(9), None);
    }
}
