//! The interface every speech model implements.
//!
//! Deliberately narrow: hand it audio, get text back. Sessions own all the timing, buffering and
//! segmentation logic, so adding a runtime means implementing one method rather than reproducing a
//! state machine.

use summo_core::{Result, segment::Word};

/// One decode result.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub text: String,
    /// Model confidence, where the runtime exposes one.
    pub confidence: Option<f32>,
    /// Whisper-family models report how likely the audio was silence. High values combined with
    /// confident text are the signature of a hallucination.
    pub no_speech_prob: Option<f32>,
    /// Word timings, where the runtime provides them. Drives click-to-seek and karaoke highlight.
    pub words: Vec<Word>,
    /// The language the runtime says it heard, as a bare ISO code.
    ///
    /// Only the multilingual runtimes answer, and only when they were asked to detect rather than
    /// told. `None` therefore means "this decoder does not know", which is not the same as "the
    /// audio had no language" — so anything routing on this has to have an answer for `None` that
    /// is not "skip it".
    ///
    /// Here rather than inferred from the text because guessing a language from a sentence is a
    /// model in its own right, and the one that just ran already knows.
    pub language: Option<String>,
}

impl Transcript {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// A speech-to-text model.
///
/// Implementations are called from one thread at a time and may hold heavy state (ONNX sessions,
/// KV caches). `decode` receives the *whole* utterance so far, not an increment — batch models
/// require that, and streaming models that prefer increments can keep their own cursor.
pub trait Decoder: Send {
    /// Decode a complete utterance's audio at 16 kHz mono.
    fn decode(&mut self, pcm: &[f32]) -> Result<Transcript>;

    /// Drop any per-utterance state. Called when a segment closes.
    fn reset(&mut self) {}

    fn name(&self) -> &str;

    /// Whether this decoder can produce useful text from a partially spoken utterance.
    ///
    /// Streaming models say yes. Whisper-family models technically return *something* for a
    /// half-utterance, and it is often a confidently wrong guess at the ending, so a decoder can opt
    /// out of partials and let the session show text only when the utterance is complete.
    fn supports_partials(&self) -> bool {
        true
    }
}

/// Lets a boxed decoder be used wherever a concrete one is expected.
///
/// Needed because the runtime is chosen at startup from configuration, so the concrete type is not
/// known until then, while sessions are generic over it for the sake of static dispatch in the hot
/// loop.
impl<D: Decoder + ?Sized> Decoder for Box<D> {
    fn decode(&mut self, pcm: &[f32]) -> Result<Transcript> {
        (**self).decode(pcm)
    }

    fn reset(&mut self) {
        (**self).reset();
    }

    fn name(&self) -> &str {
        (**self).name()
    }

    fn supports_partials(&self) -> bool {
        (**self).supports_partials()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A decoder that returns a prefix of a fixed sentence proportional to the audio it is given,
    /// which is how a real streaming model behaves and lets sessions be tested without a model.
    pub struct GrowingDecoder {
        pub sentence: Vec<String>,
        /// Audio duration, in seconds, that "reveals" one more word.
        pub secs_per_word: f64,
        pub calls: usize,
        pub supports_partials: bool,
        pub no_speech_prob: Option<f32>,
    }

    impl GrowingDecoder {
        pub fn new(sentence: &str) -> Self {
            Self {
                sentence: sentence.split_whitespace().map(str::to_string).collect(),
                secs_per_word: 0.2,
                calls: 0,
                supports_partials: true,
                no_speech_prob: None,
            }
        }
    }

    impl Decoder for GrowingDecoder {
        fn decode(&mut self, pcm: &[f32]) -> Result<Transcript> {
            self.calls += 1;
            let secs = pcm.len() as f64 / f64::from(summo_core::SAMPLE_RATE);
            let words = ((secs / self.secs_per_word) as usize).min(self.sentence.len());
            Ok(Transcript {
                text: self.sentence[..words].join(" "),
                confidence: Some(0.9),
                no_speech_prob: self.no_speech_prob,
                words: Vec::new(),
                language: None,
            })
        }

        fn name(&self) -> &str {
            "growing"
        }

        fn supports_partials(&self) -> bool {
            self.supports_partials
        }
    }

    /// A decoder that always returns the same text, for tests that care about call counts.
    pub struct FixedDecoder {
        pub text: String,
        pub calls: usize,
    }

    impl FixedDecoder {
        pub fn new(text: &str) -> Self {
            Self {
                text: text.into(),
                calls: 0,
            }
        }
    }

    impl Decoder for FixedDecoder {
        fn decode(&mut self, _pcm: &[f32]) -> Result<Transcript> {
            self.calls += 1;
            Ok(Transcript::new(self.text.clone()))
        }

        fn name(&self) -> &str {
            "fixed"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{test_support::GrowingDecoder, *};

    #[test]
    fn transcripts_treat_whitespace_as_empty() {
        assert!(Transcript::new("   ").is_empty());
        assert!(Transcript::default().is_empty());
        assert!(!Transcript::new("xin chào").is_empty());
    }

    #[test]
    fn a_boxed_decoder_forwards_every_method() {
        let mut boxed: Box<dyn Decoder> = Box::new(test_support::FixedDecoder::new("xong"));
        assert_eq!(boxed.decode(&[0.0; 16]).unwrap().text, "xong");
        assert_eq!(boxed.name(), "fixed");
        assert!(boxed.supports_partials());
        boxed.reset();
    }

    #[test]
    fn the_test_decoder_reveals_words_as_audio_grows() {
        let mut d = GrowingDecoder::new("một hai ba bốn");
        let sr = summo_core::SAMPLE_RATE as usize;

        assert_eq!(d.decode(&vec![0.0; sr / 5]).unwrap().text, "một");
        assert_eq!(d.decode(&vec![0.0; sr * 2 / 5]).unwrap().text, "một hai");
        assert_eq!(
            d.decode(&vec![0.0; sr * 10]).unwrap().text,
            "một hai ba bốn"
        );
        assert_eq!(d.calls, 3);
    }
}
