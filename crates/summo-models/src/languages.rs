//! Which languages can actually be recognised, and by what.
//!
//! The app used to ask this question backwards. Setup recommended a model for whatever language the
//! *interface* was in, on the theory that somebody reading Vietnamese is recording Vietnamese —
//! true often enough to be invisible when it is wrong, and wrong is a 73 MB download that cannot
//! transcribe the meeting it was installed for.
//!
//! So the question is asked directly, and answering it needs a list. Not a list of languages the
//! world has, and not a list somebody typed into the interface: the languages *this registry* can
//! serve, each with the model that would serve it, so a picker can say "Tiếng Việt — gipformer-65m,
//! 73 MB" and "日本語 — whisper-tiny, 256 MB, never measured on this language" and let a user see
//! the difference before they spend the download.
//!
//! ## Why `*` is expanded here
//!
//! A multilingual manifest declares `langs: ["*"]`, which is honest — Whisper really was trained on
//! ninety-nine languages — but a picker cannot render `*`. Expanding it needs a list of what the
//! multilingual models actually cover, and that list is a property of those models rather than of
//! the registry, so it lives here as data rather than in each manifest as ninety-nine strings
//! repeated per model.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{hw::HwProfile, manifest::Manifest, recommend::recommend, variant};

/// The languages a `langs: ["*"]` model is claiming.
///
/// Whisper's own list, which is also SenseVoice's superset — a model that covers fewer says so
/// explicitly rather than using `*`. Ordered by speakers rather than alphabetically so that a
/// truncated list is still a useful one.
pub const MULTILINGUAL: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
    "ha", "ba", "jw", "su", "yue",
];

/// One language a user could choose, and what would serve it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Language {
    /// ISO code, as the manifests write it.
    pub code: String,
    /// The best model for it on this machine, or `None` when nothing here covers it.
    pub model: Option<String>,
    pub model_name: Option<String>,
    /// Download size of that model, so a picker can show the cost before it is paid.
    pub size_bytes: u64,
    /// Whether that model is already on disk.
    pub installed: bool,
    /// Measured accuracy on this language, `0.0` when nobody has measured it.
    ///
    /// This is what separates "Whisper covers Vietnamese" from "Whisper is usable in Vietnamese".
    /// It is 0.345 for whisper-tiny on `vi` and 0.915 for gipformer, and a picker that hides the
    /// difference is recommending a model that will disappoint.
    pub accuracy: f32,
    /// Whether the model that serves it can keep up with live audio here.
    pub live: bool,
    /// Whether the only model covering it does so through `*` rather than by being measured on it.
    pub multilingual_only: bool,
}

/// Every language the given manifests can serve, best model first for each.
///
/// Sorted with the measured languages first and the rest alphabetically by code: a list where
/// Vietnamese is buried between `ur` and `yi` is a list nobody scrolls.
#[must_use]
pub fn available(manifests: &[Manifest], hw: &HwProfile, installed: &[String]) -> Vec<Language> {
    let mut codes: BTreeMap<String, bool> = BTreeMap::new();

    for manifest in manifests
        .iter()
        .filter(|m| m.task == crate::manifest::Task::Asr)
    {
        for lang in &manifest.langs {
            if lang == "*" {
                for code in MULTILINGUAL {
                    codes.entry((*code).to_string()).or_insert(true);
                }
            } else {
                // A language a model names explicitly is not "multilingual only", whatever else
                // covers it.
                codes.insert(lang.to_lowercase(), false);
            }
        }
    }

    let mut out: Vec<Language> = codes
        .into_iter()
        .map(|(code, multilingual_only)| {
            let ranked = recommend(manifests, hw, &code);
            let best = ranked.best();
            let manifest = best.and_then(|s| manifests.iter().find(|m| m.id.as_str() == s.id));
            Language {
                model: best.map(|s| s.id.clone()),
                model_name: best.map(|s| s.name.clone()),
                size_bytes: manifest.map_or(0, |m| download_size(m, hw)),
                installed: best.is_some_and(|s| installed.iter().any(|id| id == &s.id)),
                // Measured *on this language*, not the ranking score.
                //
                // `recommend` deliberately falls back to any benchmark a model has, because for
                // ranking, evidence from another language beats none. Reported per language that
                // becomes a lie: it claimed Whisper was 32 % accurate in Afrikaans, a number that
                // came from a Vietnamese test set. Here, unmeasured is zero and the interface says
                // "nobody has measured this".
                accuracy: manifest.map_or(0.0, |m| measured(m, &code)),
                live: best.is_some_and(|s| s.live_capable),
                multilingual_only,
                code,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        // Measured before unmeasured, then by accuracy, then by code so the order is stable.
        b.accuracy
            .partial_cmp(&a.accuracy)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.code.cmp(&b.code))
    });
    out
}

/// What this machine would actually download for a model.
///
/// The manifest's `size_bytes` is every file it publishes, and a model that ships fp32 *and* int8
/// publishes both — so quoting it in a picker overstates the download by the size of the build the
/// user will not fetch. This asks the same code the installer uses which variant applies here, and
/// adds up only those files.
fn download_size(manifest: &Manifest, hw: &HwProfile) -> u64 {
    let choice = variant::choose(manifest, hw);
    let files = variant::files(manifest, &choice);
    if files.is_empty() {
        return manifest.size_bytes;
    }
    files.iter().map(|f| f.size).sum()
}

/// Accuracy measured on one language, or `0.0` when nobody has measured it.
///
/// Matches `wer_<benchmark>_<code>` — the convention the manifests already use — and returns
/// `1 - wer`. A key for another language is not evidence about this one.
fn measured(manifest: &Manifest, code: &str) -> f32 {
    let suffix = format!("_{}", code.to_lowercase());
    manifest
        .profile
        .quality
        .iter()
        .find(|(key, _)| key.starts_with("wer") && key.to_lowercase().ends_with(&suffix))
        .map_or(0.0, |(_, wer)| (1.0 - wer).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{FileEntry, Mode, Profile, RssProfile, Task};
    use summo_core::ModelId;

    fn manifest(id: &str, langs: &[&str], wer: Option<(&str, f32)>) -> Manifest {
        let mut profile = Profile {
            rss_mb: RssProfile {
                idle: 10,
                peak: 100,
            },
            min_ram_mb: 100,
            ..Profile::default()
        };
        profile.rtf.insert("cpu_x86_avx512vnni_8t".into(), 0.02);
        if let Some((key, value)) = wer {
            profile.quality.insert(key.into(), value);
        }

        Manifest {
            schema: 1,
            id: ModelId::parse(id).unwrap(),
            name: id.to_string(),
            task: Task::Asr,
            mode: Mode::Live,
            runtime: "test".into(),
            langs: langs.iter().map(|s| (*s).to_string()).collect(),
            domains: vec![],
            license: "MIT".into(),
            attribution: None,
            redistributable: true,
            gated: false,
            size_bytes: 1_000,
            profile,
            files: vec![FileEntry {
                name: "m.onnx".into(),
                sha256: "a".repeat(64),
                size: 1_000,
                url: "https://example.invalid/m".into(),
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

    fn hw() -> HwProfile {
        let mut hw = HwProfile::detect();
        hw.available_ram_mb = 16_000;
        hw
    }

    /// The picker cannot render `*`, and a user cannot choose a language nobody lists.
    #[test]
    fn a_multilingual_model_contributes_the_languages_it_covers() {
        let all = available(
            &[manifest("whisper", &["*"], Some(("wer_x", 0.5)))],
            &hw(),
            &[],
        );
        assert!(all.iter().any(|l| l.code == "ja"));
        assert!(all.iter().any(|l| l.code == "vi"));
        assert!(all.iter().all(|l| l.multilingual_only));
    }

    /// The distinction the whole screen exists for: covered is not the same as good.
    #[test]
    fn a_language_a_model_was_measured_on_outranks_one_it_merely_covers() {
        let all = available(
            &[
                manifest("gipformer", &["vi"], Some(("wer_fleurs_vi", 0.085))),
                manifest("whisper", &["*"], Some(("wer_fleurs_vi", 0.655))),
            ],
            &hw(),
            &[],
        );
        let vi = all.iter().find(|l| l.code == "vi").expect("vi is covered");
        assert_eq!(vi.model.as_deref(), Some("gipformer"));
        assert!(
            !vi.multilingual_only,
            "vi is named by a model, not only implied"
        );
        assert!(vi.accuracy > 0.9);

        // And the first entry is a language somebody measured, not `af`.
        assert_eq!(all.first().map(|l| l.code.as_str()), Some("vi"));
    }

    #[test]
    fn installation_is_reported_per_language_because_it_is_a_property_of_the_model() {
        let manifests = [manifest(
            "gipformer",
            &["vi"],
            Some(("wer_fleurs_vi", 0.085)),
        )];
        let installed = vec!["gipformer".to_string()];
        let all = available(&manifests, &hw(), &installed);
        assert!(all.iter().find(|l| l.code == "vi").unwrap().installed);
        assert!(!available(&manifests, &hw(), &[])[0].installed);
    }

    /// A registry with no speech models at all is a valid state — a fresh install with no network —
    /// and it must produce an empty list rather than every language with no model.
    #[test]
    fn nothing_to_offer_is_an_empty_list() {
        assert!(available(&[], &hw(), &[]).is_empty());
    }

    /// The bug this caught in the running app: every one of Whisper's ninety-nine languages was
    /// reported at 32 % accurate, a number that came from the *Vietnamese* test set, because
    /// `recommend` falls back to any benchmark a model has. Good for ranking, false as a per
    /// language claim.
    #[test]
    fn accuracy_measured_on_another_language_is_not_reported_as_this_one() {
        let all = available(
            &[manifest("whisper", &["*"], Some(("wer_fleurs_vi", 0.655)))],
            &hw(),
            &[],
        );
        let vi = all.iter().find(|l| l.code == "vi").unwrap();
        let af = all.iter().find(|l| l.code == "af").unwrap();
        assert!(
            (vi.accuracy - 0.345).abs() < 0.001,
            "vi was measured: {vi:?}"
        );
        assert_eq!(af.accuracy, 0.0, "nobody has measured Afrikaans: {af:?}");
    }

    /// A picker quoting the manifest's total overstates a download by the size of the build that
    /// will not be fetched — whisper-tiny publishes fp32 and int8 and installs one of them.
    #[test]
    fn the_size_shown_is_the_variant_this_machine_would_fetch() {
        let mut m = manifest("whisper", &["*"], Some(("wer_x_en", 0.05)));
        m.size_bytes = 999_999;
        m.variants = vec![
            crate::variant::Variant {
                name: Some("int8".into()),
                accel: None,
                precision: Some(crate::variant::Precision::Int8),
            },
            crate::variant::Variant {
                name: Some("fp32".into()),
                accel: None,
                precision: Some(crate::variant::Precision::Fp32),
            },
        ];
        m.files = vec![
            FileEntry {
                name: "tokens.txt".into(),
                sha256: "a".repeat(64),
                size: 10,
                url: "https://example.invalid/t".into(),
                mirror: vec![],
                platform: None,
                variant: None,
            },
            FileEntry {
                name: "int8.onnx".into(),
                sha256: "b".repeat(64),
                size: 100,
                url: "https://example.invalid/i".into(),
                mirror: vec![],
                platform: None,
                variant: Some("int8".into()),
            },
            FileEntry {
                name: "fp32.onnx".into(),
                sha256: "c".repeat(64),
                size: 400,
                url: "https://example.invalid/f".into(),
                mirror: vec![],
                platform: None,
                variant: Some("fp32".into()),
            },
        ];

        let all = available(&[m], &hw(), &[]);
        let en = all.iter().find(|l| l.code == "en").unwrap();
        // Plenty of memory here, so fp32 wins: 400 + the untagged 10, and not 999,999 or 510.
        assert_eq!(en.size_bytes, 410, "{en:?}");
    }
}
