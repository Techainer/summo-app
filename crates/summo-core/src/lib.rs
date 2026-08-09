//! Shared vocabulary for every Summo crate.
//!
//! Nothing in here may depend on a runtime, an OS API or a model format — this crate is compiled
//! for desktop, iOS and Android alike, so it stays free of platform-specific dependencies.

pub mod audio;
pub mod error;
pub mod event;
pub mod ids;
pub mod paths;
pub mod segment;

pub use audio::{FRAME_LEN, FRAME_MS, SAMPLE_RATE};
pub use error::{Error, Result};
pub use event::{Event, ModelProgress, ProgressStage, Stat};
pub use ids::{MeetingId, ModelId, SpeakerId};
pub use segment::{Lane, Segment, SegmentSource, Word};
