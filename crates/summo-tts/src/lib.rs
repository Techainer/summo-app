//! Speech synthesis, and the harder half: putting a translated line back where it came from.
//!
//! Dubbing is not "synthesise each line and concatenate". A meeting has *slots* — the original
//! speaker started at 12.0s and stopped at 14.5s — and the dubbed line has to live inside its slot,
//! or the whole track walks away from the video. English is routinely 20–30% shorter than
//! Vietnamese for the same content and Japanese is routinely longer, so almost every line needs
//! either a stretch or a squeeze, and the interesting question is what to do when the squeeze would
//! be so severe that the result is unintelligible.
//!
//! That question is [`plan`], and it is the part of this crate worth having tests for. Synthesis
//! itself is a model behind a trait.
//!
//! ## Why the model is a trait rather than a hard dependency
//!
//! Vietnamese TTS is a licensing minefield: viXTTS and XTTS-v2 are Coqui CPML and Coqui is defunct,
//! so no licence can be bought; F5-TTS-Vietnamese and valtec are CC-BY-NC; VietTTS has Apache code
//! and CC-BY-NC *weights*. **VieNeu-TTS is Apache-2.0**, which is why it is the default — and it is
//! the best of them anyway.
//!
//! But a user is allowed to point Summo at a non-commercial model they installed themselves: the
//! distinction that matters is who distributes the weights, not who runs them. A trait is what
//! makes that a configuration choice instead of a fork.

pub mod dub;
pub mod plan;

#[cfg(feature = "vieneu")]
pub mod vieneu;

pub use plan::{Fit, Line, Plan, Slot, plan};

use summo_core::Result;

/// Audio produced by a synthesiser.
pub struct Speech {
    pub samples: Vec<f32>,
    pub rate: u32,
}

impl Speech {
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        if self.rate == 0 {
            return 0.0;
        }
        self.samples.len() as f64 / f64::from(self.rate)
    }
}

/// Printed as a shape. A minute of speech is a million floats and one stray `{:?}` in a log line
/// would be a megabyte of noise.
impl std::fmt::Debug for Speech {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Speech")
            .field("samples", &self.samples.len())
            .field("rate", &self.rate)
            .finish()
    }
}

/// A voice to speak in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Voice {
    /// A built-in voice's name, if the model has any.
    pub name: Option<String>,
    /// A 3–8 second reference clip to clone from.
    ///
    /// Cloning a *participant's* voice from their own meeting is the obvious use and also the one
    /// that needs consent, which is why the caller supplies the path rather than this crate going
    /// looking for one in the vault.
    pub reference: Option<std::path::PathBuf>,
}

/// Anything that can turn text into speech.
pub trait Synthesizer: Send {
    /// Sample rate of everything this produces.
    fn rate(&self) -> u32;

    /// Speak one line.
    fn say(&mut self, text: &str, voice: &Voice) -> Result<Speech>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_comes_from_the_samples_and_the_rate() {
        let speech = Speech {
            samples: vec![0.0; 24_000],
            rate: 48_000,
        };
        assert!((speech.duration_s() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_zero_rate_does_not_divide_by_zero() {
        let speech = Speech {
            samples: vec![0.0; 10],
            rate: 0,
        };
        assert_eq!(speech.duration_s(), 0.0);
    }

    #[test]
    fn debug_prints_the_shape_not_the_samples() {
        let speech = Speech {
            samples: vec![0.5; 1_000],
            rate: 48_000,
        };
        let text = format!("{speech:?}");
        assert!(text.contains("1000"));
        assert!(!text.contains("0.5"));
    }
}
