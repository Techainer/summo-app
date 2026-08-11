//! Choosing which build of a model to fetch.
//!
//! A model is rarely one file. The same weights ship as fp32 and int8, as a CoreML package for
//! Apple silicon and a CUDA-friendly export for a machine with an NVIDIA card, sometimes with a
//! separate encoder per quantisation. Fetching all of them wastes a gigabyte; fetching the wrong
//! one is worse — an int8 export on a machine with headroom loses accuracy nobody asked to trade,
//! and a CUDA export on a laptop with no GPU simply will not load.
//!
//! So a manifest may tag a file with the platform and the accelerator it is for, and this picks.
//! Nothing is inferred from filenames: a rule that reads `int8` out of a name works until somebody
//! publishes `model-int8-calibration-fp32.onnx`.
//!
//! ## What decides
//!
//! **Platform first, and it is absolute.** A `macos-arm64` file on Linux is not a worse choice, it
//! is not a choice — it cannot be loaded at all.
//!
//! **Then the accelerator this machine actually has**, best first, in the order the profile lists
//! them. `HwProfile::accel` is measured rather than guessed, which is why the ranking lives there
//! and not here.
//!
//! **Then precision, against free memory.** Quantised when the full-precision build would not fit,
//! full-precision when it would. That is the only place this trades accuracy, and it does it in the
//! direction the machine forces.

use serde::{Deserialize, Serialize};

use crate::hw::HwProfile;
use crate::manifest::{Accel, FileEntry, Manifest, current_platform};

/// How a file was quantised, when the publisher said.
///
/// Absent means unquantised or unstated, which are treated the same: a manifest that does not say
/// gets no preference either way rather than a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    /// 8-bit. Smallest and fastest; measurably less accurate.
    Int8,
    /// 16-bit float. The usual middle ground on a GPU.
    Fp16,
    /// Full precision.
    Fp32,
}

impl Precision {
    /// Roughly how much this multiplies the resident cost relative to int8.
    ///
    /// Used only to compare candidates from one manifest against one another, so being approximate
    /// is fine — being wrong in the ordering would not be.
    #[must_use]
    pub fn weight(self) -> f32 {
        match self {
            Precision::Int8 => 1.0,
            Precision::Fp16 => 2.0,
            Precision::Fp32 => 4.0,
        }
    }
}

/// A group of files that go together — one build of a model.
///
/// Files with no `variant` tag belong to every variant: a tokeniser or a config is the same
/// whichever export you fetched, and duplicating it per variant is how a manifest gets three copies
/// of the same 2 MB file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Variant {
    /// Name referenced by `Manifest::params`, e.g. `int8` or `coreml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Accelerator this build is for. `None` means it runs anywhere the platform does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accel: Option<Accel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<Precision>,
}

/// Which variant a file belongs to.
///
/// Read off the entry rather than parsed from its name. A rule that reads `int8` out of a filename
/// works right up until somebody publishes `model-int8-calibration-fp32.onnx`.
#[must_use]
pub fn variant_of(file: &FileEntry) -> Option<&str> {
    file.variant.as_deref()
}

/// Every variant a manifest offers, in the order it declared them.
#[must_use]
pub fn variants(manifest: &Manifest) -> &[Variant] {
    &manifest.variants
}

/// Why a variant was not chosen. Reported so a user can see the reasoning rather than a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub variant: String,
    pub why: String,
}

/// What to fetch on this machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// The variant name, or `None` when the manifest offers only one build.
    pub variant: Option<String>,
    pub reason: String,
    /// Builds that would also have worked here, best-ranked first.
    ///
    /// Shown, not hidden. A user who only ever sees rejections never learns that a smaller build
    /// exists, and "you can disagree with this" is not true of a choice you cannot see.
    pub alternatives: Vec<String>,
    pub rejected: Vec<Rejected>,
}

/// Pick the variant that fits this machine.
///
/// Returns `None` for a manifest with no variants at all, which is the common case and means "there
/// is one build; fetch it".
#[must_use]
pub fn choose(manifest: &Manifest, hw: &HwProfile) -> Choice {
    let mut rejected = Vec::new();

    if manifest.variants.is_empty() {
        return Choice {
            variant: None,
            reason: "one build".into(),
            alternatives: Vec::new(),
            rejected,
        };
    }

    let here = current_platform();
    let mut usable: Vec<&Variant> = Vec::new();

    for variant in &manifest.variants {
        let name = variant.name.clone().unwrap_or_else(|| "default".into());

        // A variant with no files for this platform cannot be used, however good it looks.
        let has_files = manifest
            .files
            .iter()
            .any(|f| files_match(f, variant, &here));
        if !has_files {
            rejected.push(Rejected {
                variant: name,
                why: format!("no files for {here}"),
            });
            continue;
        }

        if let Some(accel) = variant.accel
            && accel != Accel::Cpu
            && !hw.accel.contains(&accel)
        {
            rejected.push(Rejected {
                variant: name,
                why: format!("this machine has no {accel:?}"),
            });
            continue;
        }

        if !fits(manifest, variant, hw) {
            rejected.push(Rejected {
                variant: name,
                why: format!("needs more than {} MB free", hw.available_ram_mb),
            });
            continue;
        }

        usable.push(variant);
    }

    usable.sort_by(|a, b| {
        rank(b, hw)
            .partial_cmp(&rank(a, hw))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let alternatives: Vec<String> = usable
        .iter()
        .skip(1)
        .filter_map(|v| v.name.clone())
        .collect();

    let Some(best) = usable.first().copied() else {
        // Everything was refused. Falling back to the first declared build is better than refusing
        // to install: the machine may still run it slowly, and a user who asked for a model should
        // get one rather than a lecture.
        let first = manifest.variants.first();
        return Choice {
            variant: first.and_then(|v| v.name.clone()),
            reason: "nothing fits cleanly; taking the first build".into(),
            alternatives: Vec::new(),
            rejected,
        };
    };

    Choice {
        reason: describe(best, hw),
        variant: best.name.clone(),
        alternatives,
        rejected,
    }
}

/// Whether a file belongs to a variant on this platform.
fn files_match(file: &FileEntry, variant: &Variant, here: &str) -> bool {
    if file.platform.as_ref().is_some_and(|p| p != here) {
        return false;
    }
    match (&file.variant, &variant.name) {
        // Untagged files belong to every variant.
        (None, _) => true,
        (Some(tag), Some(name)) => tag == name,
        (Some(_), None) => false,
    }
}

/// Whether this variant's resident cost plausibly fits.
///
/// The manifest states one memory profile for the model; precision scales it. Approximate on
/// purpose — the alternative is a per-variant profile nobody would keep accurate.
fn fits(manifest: &Manifest, variant: &Variant, hw: &HwProfile) -> bool {
    let base = manifest
        .profile
        .min_ram_mb
        .max(manifest.profile.rss_mb.peak) as f32;
    let scaled = match variant.precision {
        // The profile is quoted for the full-precision build, so a quantised one costs less.
        Some(p) => base * (p.weight() / Precision::Fp32.weight()),
        None => base,
    };
    // Headroom: a model that exactly fills free memory leaves nothing for the rest of the app.
    scaled * 1.3 <= hw.available_ram_mb as f32
}

/// Higher is better.
fn rank(variant: &Variant, hw: &HwProfile) -> f32 {
    let mut score = 0.0;

    // A real accelerator beats the CPU by more than any precision difference is worth.
    if let Some(accel) = variant.accel
        && accel != Accel::Cpu
    {
        // Earlier in the profile's list is better; the profile ranks what this machine actually has.
        let position = hw
            .accel
            .iter()
            .position(|a| *a == accel)
            .unwrap_or(usize::MAX);
        score += 100.0 - position as f32;
    }

    // Then precision, as high as memory allows. `fits` has already excluded what does not.
    score += match variant.precision {
        Some(Precision::Fp32) => 3.0,
        Some(Precision::Fp16) => 2.0,
        Some(Precision::Int8) => 1.0,
        None => 2.5,
    };
    score
}

fn describe(variant: &Variant, hw: &HwProfile) -> String {
    let mut parts = Vec::new();
    match variant.accel {
        Some(accel) if accel != Accel::Cpu => parts.push(format!("{accel:?} on this machine")),
        _ => parts.push("cpu".into()),
    }
    if let Some(precision) = variant.precision {
        parts.push(format!("{precision:?}").to_lowercase());
    }
    parts.push(format!("{} MB free", hw.available_ram_mb));
    parts.join(", ")
}

/// The files to fetch, once a variant is chosen.
#[must_use]
pub fn files<'a>(manifest: &'a Manifest, choice: &Choice) -> Vec<&'a FileEntry> {
    let here = current_platform();
    let selected = Variant {
        name: choice.variant.clone(),
        accel: None,
        precision: None,
    };
    manifest
        .files
        .iter()
        .filter(|f| files_match(f, &selected, &here))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Mode, Profile, RssProfile, Task};
    use summo_core::ModelId;

    fn file(name: &str, variant: Option<&str>, platform: Option<&str>) -> FileEntry {
        FileEntry {
            name: name.into(),
            sha256: "a".repeat(64),
            size: 1_000,
            url: "https://example.invalid/x".into(),
            mirror: vec![],
            platform: platform.map(str::to_string),
            variant: variant.map(str::to_string),
        }
    }

    fn manifest(variants: Vec<Variant>, files: Vec<FileEntry>, ram_mb: u32) -> Manifest {
        Manifest {
            schema: 1,
            id: ModelId::parse("m").unwrap(),
            name: "M".into(),
            task: Task::Asr,
            mode: Mode::Batch,
            runtime: "r".into(),
            langs: vec![],
            domains: vec![],
            license: "MIT".into(),
            attribution: None,
            redistributable: true,
            gated: false,
            size_bytes: 0,
            description: None,
            profile: Profile {
                min_ram_mb: ram_mb,
                rss_mb: RssProfile {
                    idle: 0,
                    peak: ram_mb,
                },
                ..Profile::default()
            },
            files,
            variants,
            installed_variant: None,
            params: Default::default(),
        }
    }

    fn hw(accel: Vec<Accel>, available_ram_mb: u32) -> HwProfile {
        HwProfile {
            accel,
            available_ram_mb,
            ..HwProfile::detect()
        }
    }

    fn v(name: &str, accel: Option<Accel>, precision: Option<Precision>) -> Variant {
        Variant {
            name: Some(name.into()),
            accel,
            precision,
        }
    }

    /// The common case, and it must stay free of ceremony: one build, fetch it.
    #[test]
    fn a_manifest_with_no_variants_says_so_rather_than_choosing() {
        let m = manifest(vec![], vec![file("model.onnx", None, None)], 100);
        let choice = choose(&m, &hw(vec![Accel::Cpu], 8_000));
        assert_eq!(choice.variant, None);
        assert_eq!(files(&m, &choice).len(), 1);
    }

    /// A CUDA export on a laptop with no GPU does not load at all — this is not a preference.
    #[test]
    fn an_accelerator_this_machine_lacks_is_refused_with_a_reason() {
        let m = manifest(
            vec![
                v("cuda", Some(Accel::Cuda), None),
                v("cpu", Some(Accel::Cpu), None),
            ],
            vec![
                file("cuda.onnx", Some("cuda"), None),
                file("cpu.onnx", Some("cpu"), None),
            ],
            100,
        );
        let choice = choose(&m, &hw(vec![Accel::Cpu], 8_000));
        assert_eq!(choice.variant.as_deref(), Some("cpu"));
        assert!(
            choice
                .rejected
                .iter()
                .any(|r| r.variant == "cuda" && r.why.contains("Cuda")),
            "{:?}",
            choice.rejected
        );
    }

    #[test]
    fn an_accelerator_this_machine_has_beats_the_cpu_build() {
        let m = manifest(
            vec![
                v("cuda", Some(Accel::Cuda), None),
                v("cpu", Some(Accel::Cpu), None),
            ],
            vec![
                file("cuda.onnx", Some("cuda"), None),
                file("cpu.onnx", Some("cpu"), None),
            ],
            100,
        );
        let choice = choose(&m, &hw(vec![Accel::Cuda, Accel::Cpu], 8_000));
        assert_eq!(choice.variant.as_deref(), Some("cuda"));
    }

    /// An int8 export on a machine with headroom loses accuracy nobody asked to trade.
    #[test]
    fn full_precision_wins_when_there_is_memory_for_it() {
        let m = manifest(
            vec![
                v("int8", None, Some(Precision::Int8)),
                v("fp32", None, Some(Precision::Fp32)),
            ],
            vec![
                file("int8.onnx", Some("int8"), None),
                file("fp32.onnx", Some("fp32"), None),
            ],
            2_000,
        );
        assert_eq!(
            choose(&m, &hw(vec![Accel::Cpu], 16_000)).variant.as_deref(),
            Some("fp32")
        );
    }

    /// The only place accuracy is traded, and the machine forces it.
    #[test]
    fn quantised_wins_when_full_precision_would_not_fit() {
        let m = manifest(
            vec![
                v("int8", None, Some(Precision::Int8)),
                v("fp32", None, Some(Precision::Fp32)),
            ],
            vec![
                file("int8.onnx", Some("int8"), None),
                file("fp32.onnx", Some("fp32"), None),
            ],
            4_000,
        );
        let choice = choose(&m, &hw(vec![Accel::Cpu], 2_000));
        assert_eq!(choice.variant.as_deref(), Some("int8"));
        assert!(choice.rejected.iter().any(|r| r.variant == "fp32"));
    }

    /// A tokeniser is the same file whichever export you fetched; duplicating it per variant is how
    /// a manifest ends up with three copies of the same 2 MB file.
    #[test]
    fn untagged_files_come_with_every_variant() {
        let m = manifest(
            vec![v("int8", None, Some(Precision::Int8))],
            vec![
                file("tokens.txt", None, None),
                file("model.int8.onnx", Some("int8"), None),
            ],
            100,
        );
        let choice = choose(&m, &hw(vec![Accel::Cpu], 8_000));
        let names: Vec<&str> = files(&m, &choice).iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["tokens.txt", "model.int8.onnx"]);
    }

    /// Platform is absolute: a macOS file on Linux is not a worse choice, it is not a choice.
    #[test]
    fn a_variant_with_no_files_for_this_platform_is_unusable() {
        let m = manifest(
            vec![
                v("coreml", Some(Accel::CoreMl), None),
                v("cpu", Some(Accel::Cpu), None),
            ],
            vec![
                file("model.mlpackage", Some("coreml"), Some("macos-arm64")),
                file("model.onnx", Some("cpu"), None),
            ],
            100,
        );
        let choice = choose(&m, &hw(vec![Accel::CoreMl, Accel::Cpu], 8_000));
        if current_platform() == "macos-arm64" {
            assert_eq!(choice.variant.as_deref(), Some("coreml"));
        } else {
            assert_eq!(choice.variant.as_deref(), Some("cpu"));
            assert!(choice.rejected.iter().any(|r| r.why.contains("no files")));
        }
    }

    /// A user who asked for a model should get one, not a lecture. Slow beats absent.
    #[test]
    fn a_machine_that_fits_nothing_still_gets_a_build() {
        let m = manifest(
            vec![v("fp32", None, Some(Precision::Fp32))],
            vec![file("model.onnx", Some("fp32"), None)],
            64_000,
        );
        let choice = choose(&m, &hw(vec![Accel::Cpu], 512));
        assert_eq!(choice.variant.as_deref(), Some("fp32"));
        assert!(choice.reason.contains("nothing fits"));
        assert!(!choice.rejected.is_empty(), "and it says why");
    }

    /// The reason is shown to a user, so it has to name what actually decided.
    #[test]
    fn the_reason_names_the_accelerator_and_the_memory() {
        let m = manifest(
            vec![v("cuda", Some(Accel::Cuda), Some(Precision::Fp16))],
            vec![file("m.onnx", Some("cuda"), None)],
            100,
        );
        let choice = choose(&m, &hw(vec![Accel::Cuda, Accel::Cpu], 12_345));
        assert!(choice.reason.contains("Cuda"), "{}", choice.reason);
        assert!(choice.reason.contains("fp16"), "{}", choice.reason);
        assert!(choice.reason.contains("12345"), "{}", choice.reason);
    }

    /// "You can disagree with this" is not true of a choice you cannot see.
    #[test]
    fn the_builds_that_would_also_have_worked_are_listed() {
        let m = manifest(
            vec![
                v("int8", None, Some(Precision::Int8)),
                v("fp32", None, Some(Precision::Fp32)),
            ],
            vec![
                file("int8.onnx", Some("int8"), None),
                file("fp32.onnx", Some("fp32"), None),
            ],
            1_000,
        );
        let choice = choose(&m, &hw(vec![Accel::Cpu], 32_000));
        assert_eq!(choice.variant.as_deref(), Some("fp32"));
        assert_eq!(
            choice.alternatives,
            ["int8"],
            "the smaller build is offered, not hidden"
        );
    }

    /// Nothing is read out of a filename. `model-int8-calibration-fp32.onnx` is the case that
    /// breaks every parser somebody writes for this.
    #[test]
    fn the_variant_comes_from_the_manifest_not_the_filename() {
        let entry = file("model-int8-calibration-fp32.onnx", Some("fp32"), None);
        assert_eq!(variant_of(&entry), Some("fp32"));
    }
}
