//! Speech recognition accuracy and cost.
//!
//! Word error rate is the headline, but on its own it decides nothing: a model that is two points
//! better and three times slower cannot run live. So every run reports WER, CER and real-time
//! factor together, measured over the same audio.
//!
//! Text normalisation matters more than it sounds. A transducer emits uppercase without
//! punctuation while a reference transcript has both; scoring that as substitutions would add
//! several points of error nobody can hear, and would rank models by how they format rather than by
//! what they hear.

use std::{path::Path, time::Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use summo_asr::Decoder;
use summo_core::audio::samples_to_secs;

use crate::dataset::read_wav;

/// One labelled utterance.
#[derive(Debug, Clone, Deserialize)]
pub struct AsrItem {
    pub wav: String,
    pub text: String,
    #[serde(default)]
    pub duration_s: f64,
}

/// Accuracy and cost over a dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsrMetrics {
    /// Word error rate: (substitutions + insertions + deletions) / reference words.
    pub wer: f64,
    /// Character error rate. More informative than WER for Vietnamese, where a single wrong tone
    /// mark turns one word into another but leaves most characters right.
    pub cer: f64,
    pub rtf: f64,
    pub audio_secs: f64,
    pub decode_ms: f64,
    pub items: usize,
    /// Utterances the model returned nothing for. A high count with a low WER means the model is
    /// silently skipping hard audio rather than getting it right.
    pub empty: usize,
}

/// One model's run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsrReport {
    pub model: String,
    pub dataset: String,
    pub threads: usize,
    pub metrics: AsrMetrics,
}

impl AsrReport {
    #[must_use]
    pub fn to_markdown(reports: &[Self]) -> String {
        let mut out = String::new();
        out.push_str("| Model | Dataset | Threads | WER | CER | RTF | Audio | Empty |\n");
        out.push_str("|---|---|---:|---:|---:|---:|---:|---:|\n");
        for r in reports {
            let m = &r.metrics;
            out.push_str(&format!(
                "| {} | {} | {} | {:.1}% | {:.1}% | {:.4} | {:.1}s | {} |\n",
                r.model,
                r.dataset,
                r.threads,
                m.wer * 100.0,
                m.cer * 100.0,
                m.rtf,
                m.audio_secs,
                m.empty,
            ));
        }
        out
    }
}

/// Load a dataset directory containing `transcripts.json` and the WAV files it names.
pub fn load_items(dir: &Path) -> Result<Vec<AsrItem>> {
    let manifest = dir.join("transcripts.json");
    let body = std::fs::read_to_string(&manifest)
        .with_context(|| format!("cannot read {}", manifest.display()))?;
    let items: Vec<AsrItem> = serde_json::from_str(&body)
        .with_context(|| format!("cannot parse {}", manifest.display()))?;
    Ok(items)
}

/// Decode every item and score it.
///
/// Decoding is one pass per utterance, not the pseudo-streaming loop: this measures the model, and
/// mixing in the session's re-decode multiplier would measure the cadence setting instead.
pub fn evaluate(decoder: &mut dyn Decoder, dir: &Path, items: &[AsrItem]) -> Result<AsrMetrics> {
    let (mut word_errors, mut words) = (0_usize, 0_usize);
    let (mut char_errors, mut chars) = (0_usize, 0_usize);
    let mut audio_secs = 0.0;
    let mut decode = std::time::Duration::ZERO;
    let mut empty = 0;

    for item in items {
        let pcm = read_wav(&dir.join(&item.wav))?;

        let start = Instant::now();
        let hypothesis = decoder.decode(&pcm)?;
        decode += start.elapsed();
        decoder.reset();

        if hypothesis.is_empty() {
            empty += 1;
        }

        let reference = normalize(&item.text);
        let hypothesis = normalize(&hypothesis.text);

        let ref_words: Vec<&str> = reference.split_whitespace().collect();
        let hyp_words: Vec<&str> = hypothesis.split_whitespace().collect();
        word_errors += edit_distance(&ref_words, &hyp_words);
        words += ref_words.len();

        let ref_chars: Vec<char> = reference.chars().filter(|c| !c.is_whitespace()).collect();
        let hyp_chars: Vec<char> = hypothesis.chars().filter(|c| !c.is_whitespace()).collect();
        char_errors += edit_distance(&ref_chars, &hyp_chars);
        chars += ref_chars.len();

        audio_secs += samples_to_secs(pcm.len());
    }

    Ok(AsrMetrics {
        wer: ratio(word_errors, words),
        cer: ratio(char_errors, chars),
        rtf: if audio_secs > 0.0 {
            decode.as_secs_f64() / audio_secs
        } else {
            0.0
        },
        audio_secs,
        decode_ms: decode.as_secs_f64() * 1000.0,
        items: items.len(),
        empty,
    })
}

fn ratio(errors: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        errors as f64 / total as f64
    }
}

/// Normalise transcript text before scoring.
///
/// Lowercases, drops punctuation and collapses whitespace. None of those differences are
/// recognition errors, and counting them as such would rank models by how they format rather than
/// by what they hear.
///
/// Not handled yet: Vietnamese tone marks have two Unicode encodings (precomposed and combining),
/// and this compares them as different characters. Both our reference data and sherpa-onnx emit the
/// precomposed form, so it does not affect the numbers here — but a model that emits the combining
/// form would score far worse than it deserves. Add NFC normalisation before scoring one.
#[must_use]
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim_end().to_string()
}

/// Levenshtein distance, used for both word- and character-level scoring.
fn edit_distance<T: PartialEq>(reference: &[T], hypothesis: &[T]) -> usize {
    if reference.is_empty() {
        return hypothesis.len();
    }
    if hypothesis.is_empty() {
        return reference.len();
    }

    // Two rows are enough; the full matrix would be megabytes on a long utterance.
    let mut prev: Vec<usize> = (0..=hypothesis.len()).collect();
    let mut curr = vec![0_usize; hypothesis.len() + 1];

    for (i, r) in reference.iter().enumerate() {
        curr[0] = i + 1;
        for (j, h) in hypothesis.iter().enumerate() {
            let cost = usize::from(r != h);
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[hypothesis.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_scores_zero() {
        let words: Vec<&str> = "một hai ba".split(' ').collect();
        assert_eq!(edit_distance(&words, &words), 0);
    }

    #[test]
    fn edit_distance_counts_each_operation_once() {
        let reference: Vec<&str> = "một hai ba bốn".split(' ').collect();
        // One substitution.
        let sub: Vec<&str> = "một hai BA bốn".split(' ').collect();
        assert_eq!(edit_distance(&reference, &sub), 1);
        // One deletion.
        let del: Vec<&str> = "một hai bốn".split(' ').collect();
        assert_eq!(edit_distance(&reference, &del), 1);
        // One insertion.
        let ins: Vec<&str> = "một hai ba ba bốn".split(' ').collect();
        assert_eq!(edit_distance(&reference, &ins), 1);
    }

    #[test]
    fn an_empty_hypothesis_costs_every_reference_word() {
        let reference: Vec<&str> = "một hai ba".split(' ').collect();
        let empty: Vec<&str> = Vec::new();
        assert_eq!(edit_distance(&reference, &empty), 3);
    }

    #[test]
    fn casing_and_punctuation_are_not_recognition_errors() {
        // A transducer emits uppercase without punctuation; the reference has both.
        assert_eq!(
            normalize("Mặt khác, băng và tuyết là hiện tượng bình thường."),
            normalize("MẶT KHÁC BĂNG VÀ TUYẾT LÀ HIỆN TƯỢNG BÌNH THƯỜNG")
        );
    }

    #[test]
    fn normalisation_collapses_whitespace() {
        assert_eq!(normalize("  một   hai  "), "một hai");
        assert_eq!(normalize("!!!"), "");
    }

    #[test]
    fn character_scoring_is_gentler_than_word_scoring_on_a_tone_slip() {
        // "bàn" vs "bán" is one wrong word but only one wrong character out of three — the reason
        // both numbers are reported for Vietnamese.
        let ref_words: Vec<&str> = "cái bàn".split(' ').collect();
        let hyp_words: Vec<&str> = "cái bán".split(' ').collect();
        let word_errors = edit_distance(&ref_words, &hyp_words);

        let ref_chars: Vec<char> = "cáibàn".chars().collect();
        let hyp_chars: Vec<char> = "cáibán".chars().collect();
        let char_errors = edit_distance(&ref_chars, &hyp_chars);

        let wer = word_errors as f64 / ref_words.len() as f64;
        let cer = char_errors as f64 / ref_chars.len() as f64;
        assert!(cer < wer, "cer {cer} should be below wer {wer}");
    }

    #[test]
    fn ratio_of_nothing_is_zero_not_a_division_by_zero() {
        assert_eq!(ratio(0, 0), 0.0);
        assert_eq!(ratio(1, 4), 0.25);
    }

    #[test]
    fn markdown_reports_wer_and_rtf_together() {
        let report = AsrReport {
            model: "gipformer-65m".into(),
            dataset: "fleurs_vi".into(),
            threads: 4,
            metrics: AsrMetrics {
                wer: 0.024,
                cer: 0.013,
                rtf: 0.017,
                audio_secs: 146.6,
                decode_ms: 2492.0,
                items: 16,
                empty: 0,
            },
        };
        let md = AsrReport::to_markdown(&[report]);
        assert!(md.contains("2.4%"));
        assert!(
            md.contains("0.0170"),
            "rtf must be visible next to accuracy: {md}"
        );
    }
}
