//! Suppressing text a model invented.
//!
//! Whisper-family models were trained on subtitle data, so given silence or noise they do not
//! return nothing — they return the most common thing subtitles say over silence: "Thank you.",
//! "Subtitles by …", "♪♪♪", or the previous phrase repeated until the window fills. On a meeting
//! recording, where most audio is *not* speech, this is the single most visible failure mode.
//!
//! Three signals catch nearly all of it, and all three are needed:
//!
//! * a blacklist of subtitle boilerplate,
//! * repetition, which is what a stuck decoder produces,
//! * the model's own `no_speech_prob`, which is reliable when it is high but says nothing useful on
//!   its own for short utterances.

use crate::decoder::Transcript;

/// Subtitle boilerplate, lowercased and punctuation-stripped before comparison.
///
/// Kept short on purpose: every entry is a phrase a person might legitimately say, so a long list
/// starts deleting real speech. These are the ones that appear over *silence*, which the
/// `no_speech_prob` check qualifies.
const BOILERPLATE: &[&str] = &[
    "thank you",
    "thanks for watching",
    "thank you for watching",
    "subtitles by the amara org community",
    "subs by www zeoranger co uk",
    "please subscribe",
    "bye",
    "you",
    "hãy subscribe cho kênh",
    "ghiền mì gõ",
    "cảm ơn các bạn đã theo dõi",
    "hẹn gặp lại các bạn",
];

/// Why a transcript was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Text is plausible; emit it.
    Keep,
    /// Known subtitle boilerplate over near-silence.
    Boilerplate,
    /// The same token or phrase repeated past the point of plausibility.
    Repetition,
    /// The model itself reported the audio was almost certainly not speech.
    NoSpeech,
    /// Nothing but punctuation or whitespace.
    Empty,
}

impl Verdict {
    #[must_use]
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

/// Configurable thresholds for [`HallucinationFilter`].
#[derive(Debug, Clone, Copy)]
pub struct FilterConfig {
    /// `no_speech_prob` above which output is discarded outright.
    pub no_speech_max: f32,
    /// Softer threshold used together with the boilerplate list: a stock phrase is only suspicious
    /// when the model also thought the audio was probably silence.
    pub boilerplate_no_speech_min: f32,
    /// How many consecutive repeats of the same token are tolerated.
    pub max_token_repeats: usize,
    /// Fraction of the text a single repeated phrase may occupy before it is called a loop.
    pub max_repeat_ratio: f64,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            no_speech_max: 0.90,
            boilerplate_no_speech_min: 0.40,
            max_token_repeats: 4,
            max_repeat_ratio: 0.75,
        }
    }
}

/// Rejects invented text before it reaches the transcript.
#[derive(Debug, Clone, Default)]
pub struct HallucinationFilter {
    cfg: FilterConfig,
}

impl HallucinationFilter {
    #[must_use]
    pub fn new(cfg: FilterConfig) -> Self {
        Self { cfg }
    }

    /// Judge one decode result.
    pub fn judge(&self, transcript: &Transcript) -> Verdict {
        let normalized = normalize(&transcript.text);
        if normalized.is_empty() {
            return Verdict::Empty;
        }

        if let Some(p) = transcript.no_speech_prob {
            if p >= self.cfg.no_speech_max {
                return Verdict::NoSpeech;
            }
            // Boilerplate is only rejected when the model also doubted there was speech. Somebody
            // really can just say "thank you", and deleting that would be worse than the disease.
            if p >= self.cfg.boilerplate_no_speech_min && BOILERPLATE.contains(&normalized.as_str())
            {
                return Verdict::Boilerplate;
            }
        } else if BOILERPLATE.contains(&normalized.as_str()) {
            // Without a no-speech signal, fall back to the list alone. Losing an isolated "thank
            // you" costs less than emitting one over every silent stretch of a long meeting.
            return Verdict::Boilerplate;
        }

        if is_looping(
            &normalized,
            self.cfg.max_token_repeats,
            self.cfg.max_repeat_ratio,
        ) {
            return Verdict::Repetition;
        }

        Verdict::Keep
    }

    /// Convenience wrapper: `Some(transcript)` if it survives, `None` if it does not.
    pub fn filter(&self, transcript: Transcript) -> Option<Transcript> {
        if self.judge(&transcript).is_keep() {
            Some(transcript)
        } else {
            None
        }
    }
}

/// Lowercase, strip punctuation, collapse whitespace — so `"Thank you!!"` and `"thank  you"` compare
/// equal to the blacklist.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
            last_was_space = false;
        } else if !last_was_space {
            out.push(' ');
            last_was_space = true;
        }
    }
    out.trim_end().to_string()
}

/// Detect a stuck decoder.
///
/// Two shapes matter: the same token repeated many times in a row (`"ừ ừ ừ ừ ừ"`), and a short
/// phrase that dominates the output (`"vâng ạ vâng ạ vâng ạ"`). Natural speech does repeat, so both
/// checks need enough evidence before firing.
fn is_looping(normalized: &str, max_token_repeats: usize, max_repeat_ratio: f64) -> bool {
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    if tokens.len() < 4 {
        return false;
    }

    let mut run = 1;
    for pair in tokens.windows(2) {
        if pair[0] == pair[1] {
            run += 1;
            if run > max_token_repeats {
                return true;
            }
        } else {
            run = 1;
        }
    }

    // Phrase-level: try short cycle lengths and see whether one tiles most of the output.
    for cycle in 1..=4.min(tokens.len() / 3) {
        let pattern = &tokens[..cycle];
        let repeats = tokens
            .chunks(cycle)
            .take_while(|chunk| *chunk == pattern)
            .count();
        if repeats >= 3 && (repeats * cycle) as f64 / tokens.len() as f64 >= max_repeat_ratio {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_no_speech(text: &str, p: f32) -> Transcript {
        Transcript {
            text: text.into(),
            no_speech_prob: Some(p),
            ..Transcript::default()
        }
    }

    #[test]
    fn real_speech_is_kept() {
        let filter = HallucinationFilter::default();
        for text in [
            "Anh nghĩ mình nên dùng Rust cho phần lõi",
            "ok let's move on to the next item",
            "vâng ạ",
        ] {
            assert_eq!(
                filter.judge(&with_no_speech(text, 0.05)),
                Verdict::Keep,
                "rejected real speech: {text}"
            );
        }
    }

    #[test]
    fn subtitle_boilerplate_over_silence_is_dropped() {
        let filter = HallucinationFilter::default();
        assert_eq!(
            filter.judge(&with_no_speech("Thank you.", 0.7)),
            Verdict::Boilerplate
        );
        assert_eq!(
            filter.judge(&with_no_speech("Subtitles by the Amara.org community", 0.6)),
            Verdict::Boilerplate
        );
    }

    #[test]
    fn someone_actually_saying_thank_you_is_kept() {
        // The model is confident there was speech, so the stock phrase is taken at face value.
        let filter = HallucinationFilter::default();
        assert_eq!(
            filter.judge(&with_no_speech("Thank you.", 0.02)),
            Verdict::Keep
        );
    }

    #[test]
    fn very_high_no_speech_probability_overrides_everything() {
        let filter = HallucinationFilter::default();
        assert_eq!(
            filter.judge(&with_no_speech("một câu hoàn toàn hợp lý", 0.97)),
            Verdict::NoSpeech
        );
    }

    #[test]
    fn stuck_token_loops_are_caught() {
        let filter = HallucinationFilter::default();
        assert_eq!(
            filter.judge(&Transcript::new("ừ ừ ừ ừ ừ ừ ừ ừ")),
            Verdict::Repetition
        );
        assert_eq!(
            filter.judge(&Transcript::new("you you you you you you")),
            Verdict::Repetition
        );
    }

    #[test]
    fn repeated_phrases_are_caught() {
        let filter = HallucinationFilter::default();
        assert_eq!(
            filter.judge(&Transcript::new("vâng ạ vâng ạ vâng ạ vâng ạ")),
            Verdict::Repetition
        );
    }

    #[test]
    fn natural_repetition_survives() {
        let filter = HallucinationFilter::default();
        for text in [
            "không không phải ý anh là thế này",
            "ok ok tôi hiểu rồi",
            "yes yes go ahead",
        ] {
            assert_eq!(
                filter.judge(&Transcript::new(text)),
                Verdict::Keep,
                "rejected natural repetition: {text}"
            );
        }
    }

    #[test]
    fn punctuation_only_output_is_empty() {
        let filter = HallucinationFilter::default();
        assert_eq!(filter.judge(&Transcript::new("... !!")), Verdict::Empty);
        assert_eq!(filter.judge(&Transcript::new("   ")), Verdict::Empty);
        assert_eq!(filter.judge(&Transcript::new("♪♪♪")), Verdict::Empty);
    }

    #[test]
    fn normalization_ignores_case_and_punctuation() {
        assert_eq!(normalize("Thank  You!!"), "thank you");
        assert_eq!(normalize("Xin chào, các bạn."), "xin chào các bạn");
        assert_eq!(normalize("!!!"), "");
    }

    #[test]
    fn filter_returns_none_for_rejected_text() {
        let filter = HallucinationFilter::default();
        assert!(filter.filter(with_no_speech("Thank you.", 0.8)).is_none());
        assert!(
            filter
                .filter(with_no_speech("hôm nay họp gì", 0.1))
                .is_some()
        );
    }

    #[test]
    fn short_utterances_are_never_called_loops() {
        // Three tokens is not enough evidence; a real "dạ dạ dạ" must survive.
        let filter = HallucinationFilter::default();
        assert_eq!(filter.judge(&Transcript::new("dạ dạ dạ")), Verdict::Keep);
    }
}
