//! Choosing a model for the machine it will run on.
//!
//! The registry is a flat list on purpose — no "basic / better / best" ladder, because no single
//! ordering is right across languages. That puts the burden here: given a machine, a language and a
//! set of candidates, produce an ordering that is right *for this user*.
//!
//! Two filters come before any scoring, and both are hard:
//!
//! * **Language.** A model that does not cover the language is not a worse choice, it is not a
//!   choice. Whisper-tiny scores 4.5 % on English and 65.5 % on Vietnamese; ranking those together
//!   by "quality" would recommend it to a Vietnamese user.
//! * **Memory.** A model that does not fit will fail to load, or worse, load and push the machine
//!   into swap mid-meeting. Refusing it up front is kinder than a crash forty minutes in.
//!
//! What survives is ranked by whether it can keep up on this hardware, and only then by accuracy.
//! A model two points better that cannot run in real time is useless for live transcription.

use serde::{Deserialize, Serialize};

use crate::{
    hw::HwProfile,
    manifest::{Manifest, Mode, Task},
};

/// Real-time factor above which a model cannot drive live transcription.
///
/// Not 1.0: the pseudo-streaming loop re-decodes the open utterance several times per second, so
/// the single-pass factor has to leave room for that multiplier plus the rest of the app.
pub const LIVE_RTF_BUDGET: f32 = 0.30;

/// Headroom kept free when checking whether a model fits.
///
/// A model that exactly fills available memory leaves nothing for the audio buffers, the UI, or the
/// operating system, and the machine starts swapping while someone is talking.
const RAM_HEADROOM: f32 = 1.4;

/// A candidate, with the reasoning attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scored {
    pub id: String,
    pub name: String,
    /// Higher is better. Only comparable within one call.
    pub score: f32,
    /// Expected real-time factor on this machine, from the closest measurement available.
    pub expected_rtf: Option<f32>,
    /// Whether it can plausibly keep up with live audio here.
    pub live_capable: bool,
    /// Measured accuracy in `0.0..=1.0`, or `0.0` when nothing has been measured.
    pub accuracy: f32,
    /// Human-readable justification, shown next to the recommendation.
    pub reason: String,
}

/// Why a candidate was excluded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rejected {
    pub id: String,
    pub reason: String,
}

/// The outcome of ranking.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    /// Best first.
    pub ranked: Vec<Scored>,
    /// Excluded candidates, so a UI can explain an absence rather than silently omitting it.
    pub rejected: Vec<Rejected>,
}

impl Recommendation {
    #[must_use]
    pub fn best(&self) -> Option<&Scored> {
        self.ranked.first()
    }

    /// The pair to run: a fast model for live text, and a slower one to refine it if the machine
    /// has room for both.
    ///
    /// Returns `None` for the refine slot when the best candidate is already the most accurate one
    /// available, or when nothing else fits — refining with a worse model would make the transcript
    /// worse on purpose.
    #[must_use]
    pub fn pair(&self) -> (Option<&Scored>, Option<&Scored>) {
        let live = self.ranked.iter().find(|s| s.live_capable).or(self.best());
        let refine = self
            .ranked
            .iter()
            .filter(|candidate| Some(candidate.id.as_str()) != live.map(|l| l.id.as_str()))
            .find(|candidate| candidate.quality_beats(live));
        (live, refine)
    }
}

impl Scored {
    /// Whether this candidate is more accurate than another, ignoring speed.
    fn quality_beats(&self, other: Option<&Self>) -> bool {
        other.is_some_and(|o| self.accuracy > o.accuracy)
    }
}

/// Rank candidates for a machine and a language.
///
/// `language` is an ISO code; `"*"` in a manifest means the model covers everything.
#[must_use]
pub fn recommend(candidates: &[Manifest], hw: &HwProfile, language: &str) -> Recommendation {
    let mut ranked = Vec::new();
    let mut rejected = Vec::new();
    let hw_key = hw.key();

    for manifest in candidates.iter().filter(|m| m.task == Task::Asr) {
        if !covers_language(manifest, language) {
            rejected.push(Rejected {
                id: manifest.id.to_string(),
                reason: format!(
                    "does not cover {language} (covers {})",
                    if manifest.langs.is_empty() {
                        "nothing declared".to_string()
                    } else {
                        manifest.langs.join(", ")
                    }
                ),
            });
            continue;
        }

        let needed = required_ram_mb(manifest);
        if needed > 0 && (needed as f32 * RAM_HEADROOM) > hw.available_ram_mb as f32 {
            rejected.push(Rejected {
                id: manifest.id.to_string(),
                reason: format!(
                    "needs about {needed} MB with headroom, and {} MB is available",
                    hw.available_ram_mb
                ),
            });
            continue;
        }

        if !hw.supports(&manifest.profile.accel) {
            rejected.push(Rejected {
                id: manifest.id.to_string(),
                reason: "no execution provider on this machine can run it".into(),
            });
            continue;
        }

        ranked.push(score(manifest, &hw_key, language));
    }

    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Recommendation { ranked, rejected }
}

/// Whether a manifest claims the language.
///
/// Public because "can this model serve that language" is asked outside the ranking too — the
/// daemon asks it when the spoken language changes, to decide whether the pinned model still makes
/// sense. Two answers to that question would eventually disagree.
#[must_use]
pub fn covers_language(manifest: &Manifest, language: &str) -> bool {
    manifest.langs.iter().any(|l| {
        l == "*" || l.eq_ignore_ascii_case(language)
            // `en-US` should match a manifest that says `en`.
            || language.split('-').next().is_some_and(|base| l.eq_ignore_ascii_case(base))
    })
}

/// Memory a model needs, taking the larger of its declared minimum and its measured peak.
fn required_ram_mb(manifest: &Manifest) -> u32 {
    manifest
        .profile
        .min_ram_mb
        .max(manifest.profile.rss_mb.peak)
}

/// Quality on the benchmark closest to the requested language.
///
/// Falls back to any quality metric present, because a model measured only on another language is
/// still better evidence than none — but a language-specific number always wins.
fn accuracy_for(manifest: &Manifest, language: &str) -> Option<f32> {
    let language = language
        .split('-')
        .next()
        .unwrap_or(language)
        .to_lowercase();
    let specific = manifest
        .profile
        .quality
        .iter()
        .find(|(key, _)| key.starts_with("wer") && key.ends_with(&language));
    let any = manifest
        .profile
        .quality
        .iter()
        .find(|(key, _)| key.starts_with("wer"));

    specific.or(any).map(|(_, wer)| 1.0 - wer)
}

fn score(manifest: &Manifest, hw_key: &str, language: &str) -> Scored {
    let rtf = manifest.rtf_for(hw_key);
    let accuracy = accuracy_for(manifest, language);
    let live_capable = rtf.is_some_and(|r| r <= LIVE_RTF_BUDGET);

    // Speed first, accuracy second, and only among models that can keep up. A model two points
    // better that cannot run in real time cannot drive live text at all.
    let speed_score = match rtf {
        Some(r) if r <= LIVE_RTF_BUDGET => 100.0 * (1.0 - (r / LIVE_RTF_BUDGET)).clamp(0.0, 1.0),
        // Unmeasured on this hardware class is treated as slow rather than fast: guessing
        // optimistically would recommend a model that then cannot keep up.
        Some(_) | None => 0.0,
    };
    let accuracy_score = accuracy.unwrap_or(0.5) * 200.0;
    let live_bonus = if live_capable { 150.0 } else { 0.0 };

    let reason = match (rtf, accuracy) {
        (Some(r), Some(a)) if live_capable => format!(
            "{:.0}% accurate, {:.0}× faster than real time on this machine",
            a * 100.0,
            1.0 / r.max(f32::EPSILON)
        ),
        (Some(r), _) if !live_capable => format!(
            "too slow for live text here (real-time factor {r:.2}); usable for re-transcribing a \
             recording afterwards"
        ),
        (None, Some(a)) => format!(
            "{:.0}% accurate, but never measured on hardware like this",
            a * 100.0
        ),
        _ => "no measurements available for this machine".into(),
    };

    Scored {
        id: manifest.id.to_string(),
        name: manifest.name.clone(),
        score: speed_score + accuracy_score + live_bonus,
        expected_rtf: rtf,
        live_capable,
        reason,
        accuracy: accuracy.unwrap_or(0.0),
    }
}

/// Whether a manifest can drive live text at all, regardless of hardware.
#[must_use]
pub fn is_live_mode(manifest: &Manifest) -> bool {
    manifest.mode == Mode::Live
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{FileEntry, Profile, RssProfile};
    use summo_core::ModelId;

    fn manifest(id: &str, langs: &[&str], rtf: f32, wer_key: &str, wer: f32, ram: u32) -> Manifest {
        let mut profile = Profile {
            rss_mb: RssProfile {
                idle: 100,
                peak: ram,
            },
            min_ram_mb: ram,
            ..Profile::default()
        };
        profile.rtf.insert("cpu_x86_avx512vnni_8t".into(), rtf);
        profile.quality.insert(wer_key.into(), wer);

        Manifest {
            schema: 1,
            id: ModelId::parse(id).unwrap(),
            name: id.to_string(),
            task: Task::Asr,
            mode: Mode::Batch,
            runtime: "test".into(),
            langs: langs.iter().map(|s| (*s).to_string()).collect(),
            domains: vec![],
            license: "MIT".into(),
            attribution: None,
            redistributable: true,
            gated: false,
            size_bytes: 1,
            profile,
            files: vec![FileEntry {
                name: "m.onnx".into(),
                sha256: "a".repeat(64),
                size: 1,
                url: "https://x/y".into(),
                mirror: vec![],
                platform: None,
                variant: None,
            }],
            variants: Vec::new(),
            installed_variant: None,
            params: Default::default(),
            description: None,
        }
    }

    fn hw(available_mb: u32) -> HwProfile {
        let mut hw = HwProfile::detect();
        hw.available_ram_mb = available_mb;
        hw.cores = 8;
        hw
    }

    /// The real case: the two models actually measured, ranked for a Vietnamese user.
    #[test]
    fn a_vietnamese_user_is_not_offered_a_model_that_is_bad_at_vietnamese() {
        let candidates = vec![
            manifest("gipformer-65m", &["vi"], 0.021, "wer_fleurs_vi", 0.024, 800),
            manifest("whisper-tiny", &["*"], 0.116, "wer_fleurs_vi", 0.655, 500),
        ];
        let out = recommend(&candidates, &hw(8000), "vi");

        assert_eq!(out.best().unwrap().id, "gipformer-65m");
        assert!(out.best().unwrap().live_capable);
    }

    #[test]
    fn a_model_that_does_not_cover_the_language_is_excluded_not_ranked_low() {
        let candidates = vec![
            manifest(
                "parakeet-en",
                &["en"],
                0.01,
                "wer_librispeech_en",
                0.05,
                600,
            ),
            manifest("gipformer-65m", &["vi"], 0.021, "wer_fleurs_vi", 0.024, 800),
        ];
        let out = recommend(&candidates, &hw(8000), "vi");

        assert_eq!(out.ranked.len(), 1);
        assert_eq!(out.rejected.len(), 1);
        assert!(out.rejected[0].reason.contains("does not cover vi"));
    }

    #[test]
    fn a_wildcard_model_covers_every_language() {
        let candidates = vec![manifest("whisper", &["*"], 0.1, "wer_x", 0.1, 500)];
        assert_eq!(recommend(&candidates, &hw(8000), "th").ranked.len(), 1);
    }

    #[test]
    fn a_regional_code_matches_its_base_language() {
        let candidates = vec![manifest("en-model", &["en"], 0.1, "wer_x", 0.1, 500)];
        assert_eq!(recommend(&candidates, &hw(8000), "en-US").ranked.len(), 1);
    }

    #[test]
    fn a_model_that_does_not_fit_is_excluded_with_the_numbers() {
        // Loading it would push the machine into swap mid-meeting.
        let candidates = vec![manifest("huge", &["vi"], 0.05, "wer_fleurs_vi", 0.01, 4000)];
        let out = recommend(&candidates, &hw(2000), "vi");

        assert!(out.ranked.is_empty());
        assert!(
            out.rejected[0].reason.contains("4000 MB"),
            "got: {}",
            out.rejected[0].reason
        );
        assert!(out.rejected[0].reason.contains("2000 MB"));
    }

    #[test]
    fn headroom_is_kept_free_rather_than_filling_memory_exactly() {
        // 1500 MB needed and 1600 available is a fit on paper and a disaster in practice.
        let candidates = vec![manifest(
            "tight",
            &["vi"],
            0.05,
            "wer_fleurs_vi",
            0.01,
            1500,
        )];
        assert!(recommend(&candidates, &hw(1600), "vi").ranked.is_empty());
        assert_eq!(recommend(&candidates, &hw(4000), "vi").ranked.len(), 1);
    }

    #[test]
    fn a_model_too_slow_for_live_loses_to_a_faster_less_accurate_one() {
        // Accuracy that arrives after the meeting is not accuracy the user experiences.
        let candidates = vec![
            manifest("slow-accurate", &["vi"], 0.9, "wer_fleurs_vi", 0.01, 1000),
            manifest("fast-decent", &["vi"], 0.05, "wer_fleurs_vi", 0.06, 500),
        ];
        let out = recommend(&candidates, &hw(8000), "vi");

        assert_eq!(out.best().unwrap().id, "fast-decent");
        assert!(!out.ranked[1].live_capable);
        assert!(out.ranked[1].reason.contains("too slow for live"));
    }

    #[test]
    fn the_slower_more_accurate_model_becomes_the_refine_pass() {
        // It cannot drive live text, but re-transcribing a finished utterance is exactly its job.
        let candidates = vec![
            manifest("slow-accurate", &["vi"], 0.9, "wer_fleurs_vi", 0.01, 1000),
            manifest("fast-decent", &["vi"], 0.05, "wer_fleurs_vi", 0.06, 500),
        ];
        let out = recommend(&candidates, &hw(8000), "vi");
        let (live, refine) = out.pair();

        assert_eq!(live.unwrap().id, "fast-decent");
        assert_eq!(refine.unwrap().id, "slow-accurate");
    }

    #[test]
    fn nothing_is_paired_for_refinement_when_it_would_make_things_worse() {
        let candidates = vec![
            manifest("best", &["vi"], 0.02, "wer_fleurs_vi", 0.02, 500),
            manifest("worse", &["vi"], 0.03, "wer_fleurs_vi", 0.20, 500),
        ];
        let out = recommend(&candidates, &hw(8000), "vi");
        let (live, refine) = out.pair();

        assert_eq!(live.unwrap().id, "best");
        assert!(
            refine.is_none(),
            "refining with a worse model would be a downgrade"
        );
    }

    #[test]
    fn an_unmeasured_model_is_assumed_slow_rather_than_fast() {
        // Guessing optimistically recommends a model that then cannot keep up, which is the worst
        // possible first impression.
        let mut unmeasured = manifest("unknown", &["vi"], 0.02, "wer_fleurs_vi", 0.02, 500);
        unmeasured.profile.rtf.clear();
        let measured = manifest("known", &["vi"], 0.05, "wer_fleurs_vi", 0.05, 500);

        let out = recommend(&[unmeasured, measured], &hw(8000), "vi");
        assert_eq!(out.best().unwrap().id, "known");
        assert!(!out.ranked[1].live_capable);
    }

    #[test]
    fn a_language_specific_score_beats_a_generic_one() {
        let mut model = manifest("multi", &["*"], 0.05, "wer_librispeech_en", 0.04, 500);
        model.profile.quality.insert("wer_fleurs_vi".into(), 0.65);

        let vi = recommend(std::slice::from_ref(&model), &hw(8000), "vi");
        let en = recommend(&[model], &hw(8000), "en");

        assert!(
            vi.best().unwrap().score < en.best().unwrap().score,
            "the same model must score worse for the language it is bad at"
        );
    }

    #[test]
    fn non_asr_models_are_not_candidates() {
        let mut vad = manifest("silero", &["*"], 0.006, "wer_x", 0.0, 50);
        vad.task = Task::Vad;
        assert!(recommend(&[vad], &hw(8000), "vi").ranked.is_empty());
    }

    #[test]
    fn recommending_from_nothing_is_not_a_panic() {
        let out = recommend(&[], &hw(8000), "vi");
        assert!(out.best().is_none());
        assert_eq!(out.pair(), (None, None));
    }

    #[test]
    fn every_recommendation_explains_itself() {
        let candidates = vec![manifest("m", &["vi"], 0.02, "wer_fleurs_vi", 0.03, 500)];
        let out = recommend(&candidates, &hw(8000), "vi");
        let best = out.best().unwrap();

        assert!(best.reason.contains("97%"), "got: {}", best.reason);
        assert!(best.reason.contains("faster than real time"));
    }

    /// The daemon asks this when the spoken language changes, to decide whether the model somebody
    /// has pinned still makes sense. Before it did, choosing Japanese with a Vietnamese-only model
    /// pinned left the meeting to be decoded by a model that could not serve it.
    #[test]
    fn a_models_languages_are_what_it_can_serve() {
        let vietnamese = manifest("gipformer-65m", &["vi"], 0.3, "fleurs_vi", 0.09, 400);
        let everything = manifest("whisper-small", &["*"], 0.8, "fleurs_en", 0.12, 900);

        assert!(covers_language(&vietnamese, "vi"));
        assert!(!covers_language(&vietnamese, "ja"));
        // `*` is a claim on every language, which is what makes a multilingual model the fallback.
        assert!(covers_language(&everything, "ja"));
        // A region does not make it a different language.
        let english = manifest("en-only", &["en"], 0.3, "fleurs_en", 0.1, 400);
        assert!(covers_language(&english, "en-US"));
    }
}
