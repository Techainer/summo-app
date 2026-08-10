//! Who said what.
//!
//! Three tiers, cheapest first — the design principle is that most of the work should be done by
//! signals that cost nothing:
//!
//! 1. **Track prior.** The microphone lane is the local user by construction. On an online call
//!    that already separates "you" from "everyone else" with perfect accuracy and zero compute.
//! 2. **Online clustering.** Within the remote lane, each finished utterance is embedded and
//!    matched against a running set of voices. Milliseconds per utterance, after the text is
//!    already on screen. See [`cluster`].
//! 3. **Offline refinement.** When the meeting stops, re-cluster everything with the whole
//!    recording available and correct the labels that were guessed or skipped live.
//!
//! What this deliberately does not do is frame-level diarization with overlap detection. That costs
//! far more and buys accuracy on a case — two people talking at once — where the transcript is
//! unreliable anyway.

pub mod attribution;
pub mod cluster;
pub mod voices;

#[cfg(feature = "sherpa")]
pub mod embed;

pub use cluster::{Assignment, ClusterConfig, OnlineClusterer};
pub use attribution::{
    Attributor, Correction, Relabel, VoiceLog, VoiceSample, correct, relabel, resweep,
};
pub use voices::{Match, Person, Reassignment, Sample, VoiceBook};

use summo_core::{SpeakerId, segment::Lane};

/// The speaker a lane implies before any model runs.
///
/// This is tier one, and on a typical online call it is the only tier that matters.
#[must_use]
pub fn lane_prior(lane: Lane) -> Option<SpeakerId> {
    lane.default_speaker()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_microphone_lane_needs_no_model() {
        assert_eq!(lane_prior(Lane::Mic), Some(SpeakerId::me()));
        assert_eq!(
            lane_prior(Lane::System),
            None,
            "the remote lane is where clustering earns its keep"
        );
    }
}
