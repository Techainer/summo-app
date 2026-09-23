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

/// Bring an utterance up to the level the models were trained at.
///
/// ## What this is worth
///
/// Measured on the same hundred FLEURS clips and the same harness as every other figure in
/// `docs/benchmarks.md`, with nothing changed but the input level:
///
/// | Model | as recorded | levelled |
/// | --- | ---: | ---: |
/// | parakeet CTC 110m (en) | 51.9 %, **40 clips empty** | **8.9 %**, none empty |
/// | zipformer-gigaspeech (en) | 10.1 % | **9.2 %** |
/// | whisper-tiny fp32 (en) | 13.8 % | **13.3 %** |
/// | gipformer-65m (vi) | 8.6 % | **8.4 %** |
///
/// Every model improves; one of them goes from unusable to the best English result here. Twenty-six
/// of the hundred Vietnamese clips peak below a tenth of full scale, which is what a laptop
/// microphone across a meeting room sounds like — so this is not an exotic case, it is the ordinary
/// one.
///
/// ## Why it was found the hard way
///
/// `parakeet-tdt-110m-en` produced no text at all for 44 of a hundred clips, and the failures were
/// deterministic per clip: the same clip decoded alone, decoded ten times in a row, or decoded in a
/// batch gave the same answer every time. The clips it refused were the quiet ones — peaks of 114
/// and 198 out of 32767, against 13943 and 24533 for the clips it handled perfectly. Two different
/// exports of that model, a transducer and a CTC branch, failed on the same clips, which is what
/// ruled out the model and the runtime and left the audio.
///
/// A NeMo model normalises each mel bin over the utterance, which sounds like it should make level
/// irrelevant and does not: a signal near the log-mel floor is mostly floor, and normalising that
/// amplifies the noise rather than the speech.
///
/// ## The rules
///
/// Boost only, never attenuate: audio that is already loud is already what the model expects, and
/// pulling it down could only lose the thing that made it easy.
///
/// Capped, because the gain is applied to whatever is in the window, and an utterance the detector
/// opened on a cough is noise that would be brought up to full scale along with everything else.
/// Thirty decibels is a very quiet microphone; more than that is a microphone that is not working.
///
/// And nothing at all below [`SILENCE`], which is digital silence rather than quiet speech. There
/// is no speech in it to bring up.
#[must_use]
pub fn levelled(pcm: &[f32]) -> std::borrow::Cow<'_, [f32]> {
    let peak = pcm.iter().fold(0.0_f32, |seen, x| seen.max(x.abs()));
    if peak <= SILENCE {
        return std::borrow::Cow::Borrowed(pcm);
    }
    // Already in range. Left untouched rather than nudged to exactly the target: a copy here would
    // be one per partial re-decode, several times a second, to move audio the model already handles
    // by a fraction of a decibel.
    if peak >= LOUD_ENOUGH {
        return std::borrow::Cow::Borrowed(pcm);
    }
    let gain = (TARGET_PEAK / peak).min(MAX_GAIN);
    std::borrow::Cow::Owned(pcm.iter().map(|x| x * gain).collect())
}

/// Where a levelled utterance lands. Short of full scale, so nothing clips on the way.
const TARGET_PEAK: f32 = 0.95;

/// Most a quiet recording is lifted by: thirty decibels.
const MAX_GAIN: f32 = 32.0;

/// Loud enough to leave alone: half of full scale, six decibels down.
///
/// Everything the measurements above turned on was far below this — the clips that broke parakeet
/// peaked at 0.003 and 0.006 of full scale — so the threshold costs none of the gain and keeps the
/// ordinary case a borrow rather than a copy.
const LOUD_ENOUGH: f32 = 0.5;

/// Below this there is no speech to lift, only noise to amplify.
const SILENCE: f32 = 1e-4;

#[cfg(test)]
mod level_tests {
    use super::levelled;

    /// The case this exists for: a microphone across the room.
    #[test]
    fn a_quiet_utterance_is_brought_up() {
        let quiet: Vec<f32> = (0..100).map(|i| 0.01 * (i as f32 * 0.1).sin()).collect();
        let out = levelled(&quiet);
        let peak = out.iter().fold(0.0_f32, |s, x| s.max(x.abs()));
        assert!(peak > 0.2, "still quiet: {peak}");
        assert!(peak <= 0.96, "clipped: {peak}");
    }

    /// Audio that is already loud is left exactly as it is, byte for byte.
    ///
    /// Not merely "not attenuated": the borrowed branch is what keeps this free for the common case,
    /// and a copy taken here would be one per partial re-decode, several times a second — to move
    /// audio the model already handles by a fraction of a decibel.
    #[test]
    fn a_loud_utterance_is_not_touched() {
        for level in [0.9_f32, 0.7, 0.55] {
            let loud: Vec<f32> = (0..100).map(|i| level * (i as f32 * 0.1).sin()).collect();
            assert!(
                matches!(levelled(&loud), std::borrow::Cow::Borrowed(_)),
                "copied audio that peaks at {level}"
            );
        }
    }

    /// Silence has nothing in it to bring up, and bringing it up thirty decibels would hand the
    /// recogniser a window of amplified noise — which is what it invents text over.
    #[test]
    fn silence_is_left_silent() {
        let silence = vec![0.0_f32; 100];
        assert!(matches!(levelled(&silence), std::borrow::Cow::Borrowed(_)));

        let almost = vec![1e-6_f32; 100];
        assert!(matches!(levelled(&almost), std::borrow::Cow::Borrowed(_)));
    }

    /// And the lift is bounded, so a very quiet window is improved rather than blown up.
    #[test]
    fn the_lift_is_capped() {
        let tiny: Vec<f32> = (0..100).map(|i| 0.0005 * (i as f32).sin()).collect();
        let out = levelled(&tiny);
        let peak = out.iter().fold(0.0_f32, |s, x| s.max(x.abs()));
        assert!(peak < 0.5, "a whisper was amplified to {peak}");
    }
}
