//! Voice activity detection.
//!
//! Two separable pieces:
//!
//! * A [`Vad`] backend turns a fixed-size PCM frame into a speech probability. Backends differ
//!   mainly in how fast they notice speech *ending*, which matters more than it sounds — see
//!   [`gate`].
//! * [`VadGate`] turns that probability stream into utterance boundaries with hysteresis, a
//!   pre-roll buffer and minimum-duration guards. It is pure logic with no model dependency, so it
//!   is the part that gets tested exhaustively.

pub mod gate;

#[cfg(feature = "silero")]
pub mod silero;

#[cfg(feature = "ten-vad")]
pub mod ten;

pub use gate::{GateConfig, SpeechEvent, VadGate};

use summo_core::Result;

/// A frame-synchronous speech probability estimator.
///
/// Implementations are stateful (they carry recurrent state or a feature history) and are fed
/// contiguous, non-overlapping frames of exactly [`Vad::frame_len`] samples at 16 kHz.
pub trait Vad: Send {
    /// Samples per call. Silero wants 512 (32 ms); TEN-VAD wants 160 or 256 (10 or 16 ms).
    fn frame_len(&self) -> usize;

    /// Speech probability in `0.0..=1.0` for one frame.
    ///
    /// # Errors
    /// Returns an error if the frame length is wrong or the backend fails to run.
    fn feed_frame(&mut self, frame: &[f32]) -> Result<f32>;

    /// Drop recurrent state between sessions so one meeting cannot bias the next.
    fn reset(&mut self);

    /// Backend name for logs and benchmark rows.
    fn name(&self) -> &'static str;
}

/// A backend that always reports silence. Used by tests and by `--vad none` in the CLI.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullVad {
    frame_len: usize,
}

impl NullVad {
    #[must_use]
    pub fn new(frame_len: usize) -> Self {
        Self { frame_len }
    }
}

impl Vad for NullVad {
    fn frame_len(&self) -> usize {
        if self.frame_len == 0 {
            512
        } else {
            self.frame_len
        }
    }

    fn feed_frame(&mut self, _frame: &[f32]) -> Result<f32> {
        Ok(0.0)
    }

    fn reset(&mut self) {}

    fn name(&self) -> &'static str {
        "null"
    }
}
