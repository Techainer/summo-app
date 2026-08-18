//! The engine → client event stream.
//!
//! Serialized as JSON with an internal `kind` tag, matching the protocol in `docs/protocol.md`. The
//! desktop UI decodes this verbatim, so changing a variant is a protocol change.
//!
//! Transcript variants flatten their [`Segment`], which already carries `lane`, `seq` and the
//! timestamps — the wire form is one flat object rather than a nested one, and there is exactly one
//! place each field comes from.

use serde::{Deserialize, Serialize};

use crate::{ids::SpeakerId, segment::Segment};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// Mid-utterance text. Rendered dimmed; superseded by `Final` for the same `seq`.
    Partial(Segment),
    /// Committed on trailing silence.
    Final(Segment),
    /// A slower refine model replaced the text of an already-final segment.
    Revise(Segment),
    /// A live translation of an already-final segment.
    ///
    /// Arrives seconds after the [`Event::Final`] it belongs to — a model round trip, not a decode —
    /// so it is a separate event keyed by `seq` rather than a field on the segment. The interface
    /// renders the original immediately and slots the translation in underneath when it lands,
    /// which is what makes watching a talk in another language usable: the subtitle is late, but the
    /// transcript never is.
    Translation {
        /// The `seq` of the segment this translates.
        seq: u64,
        /// Target language tag.
        lang: String,
        text: String,
    },
    /// Offline diarization corrected a live label; the UI relabels in place.
    SpeakerRename { from: SpeakerId, to: SpeakerId },
    /// Periodic performance sample driving the HUD.
    Stat(Stat),
    /// Human-readable notice (model switched, denoise toggled, …).
    Info { text: String },
    /// Recoverable failure. Fatal errors close the connection instead.
    ///
    /// `code` is the same stable key an HTTP failure carries, so the interface can translate this
    /// the way it translates everything else. Without it the socket was the one path whose failures
    /// arrived as the daemon's own English and went straight to the status bar — `session needs a
    /// live model`, under a Vietnamese interface, the first time somebody pressed record.
    Error {
        message: String,
        transient: bool,
        /// Owned rather than `&'static str`: this is a wire type, and a borrowed field cannot be
        /// deserialised from a temporary — which is exactly what the round-trip test does.
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

impl Event {
    #[must_use]
    pub fn info(text: impl Into<String>) -> Self {
        Self::Info { text: text.into() }
    }

    #[must_use]
    pub fn error(err: &crate::Error) -> Self {
        Self::Error {
            message: err.to_string(),
            transient: err.is_transient(),
            code: err.code().map(str::to_string),
        }
    }

    /// The segment carried by transcript events, if this is one.
    #[must_use]
    pub fn segment(&self) -> Option<&Segment> {
        match self {
            Self::Partial(s) | Self::Final(s) | Self::Revise(s) => Some(s),
            _ => None,
        }
    }

    /// The lane this event belongs to, if any.
    #[must_use]
    pub fn lane(&self) -> Option<crate::segment::Lane> {
        self.segment().map(|s| s.lane)
    }
}

/// Performance sample for the HUD. Cheap to compute and emitted about once a second — this is what
/// makes a local-first app trustworthy: the user can see that it is keeping up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stat {
    /// Real-time factor: decode seconds per audio second. Below 1.0 means keeping up.
    pub rtf: f32,
    pub rss_mb: u32,
    /// Backlog waiting to be decoded, milliseconds. Growing means we are falling behind.
    pub queue_ms: u32,
}

impl Stat {
    /// True when the pipeline cannot keep up and the UI should suggest a lighter model.
    #[must_use]
    pub fn is_falling_behind(&self) -> bool {
        self.rtf >= 1.0 || self.queue_ms > 3_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::{Lane, Segment, SegmentSource};

    #[test]
    fn transcript_events_serialize_flat_with_a_kind_tag() {
        let mut s = Segment::new(3, Lane::Mic, "xin chào", 1.2, 2.0);
        s.source = SegmentSource::Final;
        let json = serde_json::to_value(Event::Final(s)).unwrap();

        assert_eq!(json["kind"], "final");
        assert_eq!(json["lane"], "mic");
        assert_eq!(json["text"], "xin chào");
        assert_eq!(json["seq"], 3);
        assert_eq!(json["speaker"], "me");
    }

    #[test]
    fn error_events_carry_retryability() {
        let err = crate::Error::Download {
            url: "https://cdn.summo.app/x".into(),
            reason: "reset".into(),
        };
        let json = serde_json::to_value(Event::error(&err)).unwrap();
        assert_eq!(json["kind"], "error");
        assert_eq!(json["transient"], true);
    }

    #[test]
    fn every_variant_round_trips() {
        let cases = vec![
            Event::Partial(Segment::new(1, Lane::System, "…", 0.0, 0.4)),
            Event::Final(Segment::new(1, Lane::Mic, "xong", 0.0, 1.0)),
            Event::Revise(Segment::new(1, Lane::Mic, "xong.", 0.0, 1.0)),
            Event::SpeakerRename {
                from: SpeakerId::auto(0),
                to: SpeakerId::auto(1),
            },
            Event::Stat(Stat {
                rtf: 0.03,
                rss_mb: 312,
                queue_ms: 18,
            }),
            Event::info("model switched"),
        ];
        for ev in cases {
            let json = serde_json::to_string(&ev).unwrap();
            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(ev, back, "round trip failed for {json}");
        }
    }

    #[test]
    fn lane_is_derived_from_the_segment_not_duplicated() {
        let ev = Event::Partial(Segment::new(0, Lane::System, "…", 0.0, 0.1));
        assert_eq!(ev.lane(), Some(Lane::System));
        assert_eq!(Event::info("x").lane(), None);
    }

    #[test]
    fn falling_behind_needs_either_rtf_or_backlog() {
        assert!(
            !Stat {
                rtf: 0.2,
                rss_mb: 300,
                queue_ms: 50
            }
            .is_falling_behind()
        );
        assert!(
            Stat {
                rtf: 1.1,
                rss_mb: 300,
                queue_ms: 50
            }
            .is_falling_behind()
        );
        assert!(
            Stat {
                rtf: 0.2,
                rss_mb: 300,
                queue_ms: 5_000
            }
            .is_falling_behind()
        );
    }
}
