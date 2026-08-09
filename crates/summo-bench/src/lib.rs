//! Measurement harness.
//!
//! Every default in Summo — which VAD ships, which ASR model suits a machine, how the gate is
//! tuned — should come from a number measured here rather than from a plausible argument. The
//! harness lives in the workspace and drives the *same* code the app runs, so a benchmark result
//! and a shipped behaviour cannot drift apart.

#[cfg(feature = "asr")]
pub mod asr;
pub mod dataset;
pub mod report;
pub mod vad;

pub use dataset::{Clip, Span, load_dataset};
pub use report::{VadMetrics, VadReport};

#[cfg(feature = "asr")]
pub use asr::{AsrMetrics, AsrReport};
