//! Turning audio into transcript segments.
//!
//! Summo has to work with two very different kinds of model, and the difference is not cosmetic:
//!
//! * **Streaming** models keep an encoder cache and cost the same per chunk no matter how long the
//!   meeting runs. They produce partial text almost immediately.
//! * **Batch** models — Whisper and friends — decode a whole utterance at once and have no notion of
//!   partial output. Naively, they can only speak after you stop talking.
//!
//! Rather than restrict the app to streaming models (which are scarcer, and weaker in Vietnamese),
//! [`PseudoSession`] drives a batch model in a loop that *re-decodes the growing utterance* every
//! few hundred milliseconds. That costs several times more compute than a single decode, which is
//! affordable precisely because these models run at real-time factors around 0.02–0.1 — there is
//! an order of magnitude of headroom, and spending it buys live text.
//!
//! [`HybridSession`] goes further: a fast model paints text immediately while a slower, more
//! accurate one re-decodes each finished utterance and replaces it. The user sees words appear at
//! streaming latency and settle into batch-model quality a moment later.

pub mod decoder;
pub mod hallucination;
pub mod hybrid;
pub mod session;

#[cfg(feature = "sherpa")]
pub mod sherpa;

pub use decoder::{Decoder, Transcript};
pub use hallucination::{HallucinationFilter, Verdict};
pub use hybrid::HybridSession;
pub use session::{PseudoSession, SessionConfig};
