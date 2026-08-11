//! What travels through a pipeline.
//!
//! One enum, deliberately. Pipecat and LiveKit Agents both settled on a single frame type flowing
//! through a chain of processors, and the reason is worth restating: with one type, a processor
//! that does not understand a frame can pass it on untouched. That is what makes the chain
//! composable — insert a translator between recognition and the sink and nothing else has to know
//! it is there, because every other stage already forwards what it does not recognise.
//!
//! The alternative, a trait object per payload, means every processor declares what it accepts and
//! the chain stops type-checking as soon as two stages disagree.
//!
//! ## Control frames are frames
//!
//! `Start`, `Flush` and `End` travel the same path as audio rather than being separate method
//! calls. A processor that buffers — the framer, the utterance gate — needs to know when the stream
//! ends, and it needs to know *in order*, after the last audio it was given. A side-channel
//! `flush()` cannot promise that ordering; a frame in the same queue can.

use summo_core::{Event, segment::Lane};

/// A block of audio, and where it came from.
#[derive(Clone, PartialEq)]
pub struct Audio {
    pub lane: Lane,
    /// Mono `f32`, in `-1.0..=1.0`, at [`Audio::rate`].
    pub samples: Vec<f32>,
    pub rate: u32,
}

impl Audio {
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        if self.rate == 0 {
            return 0.0;
        }
        self.samples.len() as f64 / f64::from(self.rate)
    }
}

/// Prints the shape, never the samples: a minute of audio is a million floats, and one stray
/// `{:?}` in a log line is a megabyte of noise.
impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio")
            .field("lane", &self.lane)
            .field("samples", &self.samples.len())
            .field("rate", &self.rate)
            .finish()
    }
}

/// Anything that flows through the chain.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// The stream is starting. Processors that hold state reset here rather than at construction,
    /// so one pipeline can serve a second recording without being rebuilt.
    Start,

    /// Audio moving downstream.
    Audio(Audio),

    /// Voice activity for the audio frame that preceded it, in `0.0..=1.0`.
    ///
    /// Separate from the audio rather than attached to it, because the detector and the decoder run
    /// at different widths: one probability describes one detector frame, and the audio it
    /// describes may already have been forwarded.
    Voice { lane: Lane, probability: f32 },

    /// Something the interface should see — a partial, a final, a stat, an error.
    ///
    /// Reusing `summo_core::Event` rather than inventing a parallel type: the wire format is
    /// already defined, and a second representation would need converting at the edge and would
    /// drift from it.
    Event(Event),

    /// Emit whatever is buffered, but keep going.
    ///
    /// Sent when a session ends and when a caller wants partial results without tearing anything
    /// down.
    Flush,

    /// The stream is over. Nothing follows.
    End,
}

impl Frame {
    #[must_use]
    pub fn audio(lane: Lane, samples: Vec<f32>, rate: u32) -> Self {
        Frame::Audio(Audio {
            lane,
            samples,
            rate,
        })
    }

    /// The lane this frame concerns, when it concerns one.
    ///
    /// Control frames belong to the whole pipeline, not to a lane, which is why they answer `None`
    /// — a lane-splitting processor has to broadcast them rather than route them.
    #[must_use]
    pub fn lane(&self) -> Option<Lane> {
        match self {
            Frame::Audio(audio) => Some(audio.lane),
            Frame::Voice { lane, .. } => Some(*lane),
            Frame::Event(event) => event.segment().map(|s| s.lane),
            _ => None,
        }
    }

    /// Whether this frame ends the stream.
    #[must_use]
    pub fn is_end(&self) -> bool {
        matches!(self, Frame::End)
    }

    /// Whether every processor must see this, whatever it is filtering for.
    ///
    /// The rule that keeps a chain correct: a processor may drop audio it has consumed, but
    /// dropping `Flush` strands whatever the next stage was buffering, and dropping `End` leaves a
    /// downstream sink waiting forever.
    #[must_use]
    pub fn is_control(&self) -> bool {
        matches!(self, Frame::Start | Frame::Flush | Frame::End)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summo_core::segment::Segment;

    #[test]
    fn audio_reports_its_own_length() {
        let audio = Audio {
            lane: Lane::Mic,
            samples: vec![0.0; 8_000],
            rate: 16_000,
        };
        assert!((audio.duration_s() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_zero_rate_does_not_divide_by_zero() {
        let audio = Audio {
            lane: Lane::Mic,
            samples: vec![0.0; 10],
            rate: 0,
        };
        assert_eq!(audio.duration_s(), 0.0);
    }

    #[test]
    fn debug_prints_the_shape_not_the_samples() {
        let frame = Frame::audio(Lane::Mic, vec![0.25; 1_000], 16_000);
        let text = format!("{frame:?}");
        assert!(text.contains("1000"));
        assert!(!text.contains("0.25"));
    }

    /// A lane-splitting processor routes by lane and must broadcast what has none.
    #[test]
    fn control_frames_belong_to_no_lane() {
        assert_eq!(Frame::Start.lane(), None);
        assert_eq!(Frame::Flush.lane(), None);
        assert_eq!(Frame::End.lane(), None);
        assert_eq!(Frame::audio(Lane::System, vec![], 16_000).lane(), Some(Lane::System));
        assert_eq!(
            Frame::Voice {
                lane: Lane::Mic,
                probability: 0.9
            }
            .lane(),
            Some(Lane::Mic)
        );
    }

    #[test]
    fn a_transcript_event_carries_the_lane_of_its_segment() {
        let event = Event::Final(Segment::new(1, Lane::System, "xin chào", 0.0, 1.0));
        assert_eq!(Frame::Event(event).lane(), Some(Lane::System));
    }

    #[test]
    fn a_notice_belongs_to_no_lane() {
        assert_eq!(Frame::Event(Event::info("hi")).lane(), None);
    }

    /// Dropping `Flush` strands whatever the next stage was buffering; dropping `End` leaves a sink
    /// waiting forever. This predicate is what every processor checks before filtering.
    #[test]
    fn control_frames_are_recognisable_as_such() {
        assert!(Frame::Start.is_control());
        assert!(Frame::Flush.is_control());
        assert!(Frame::End.is_control());
        assert!(!Frame::audio(Lane::Mic, vec![], 16_000).is_control());
        assert!(!Frame::Event(Event::info("x")).is_control());
    }

    #[test]
    fn only_end_ends_the_stream() {
        assert!(Frame::End.is_end());
        assert!(!Frame::Flush.is_end());
    }
}
