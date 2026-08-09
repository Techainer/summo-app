//! The model manifest — one JSON file fully describing one model.
//!
//! Two fields carry most of the weight:
//!
//! * [`Mode`] decides which session state machine drives the model. `Live` models stream with O(1)
//!   cost per chunk; `Batch` models are offline decoders that we drive in an aggressive
//!   pseudo-streaming loop. Getting this wrong means either no partials or a melted CPU.
//! * [`Profile`] carries measured RAM, RTF and latency, which is what lets the app recommend a
//!   model for the machine it is actually running on instead of guessing from parameter count.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use summo_core::{Error, ModelId, Result};

/// Manifest schema version. Bumped only for breaking changes; unknown newer versions are rejected
/// rather than best-effort parsed, so an old app never mis-loads a new model.
pub const SCHEMA_VERSION: u32 = 1;

/// What a model does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Task {
    Asr,
    Vad,
    Denoise,
    /// Speaker segmentation for offline diarization (pyannote-style).
    DiarizeSeg,
    /// Speaker embedding extractor for clustering / enrollment.
    SpeakerEmbed,
    /// Text embedding for retrieval.
    Embed,
}

/// How the model is driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// True streaming: cache-aware encoder, constant cost per chunk, partials in 100–300 ms.
    Live,
    /// Offline decoder. Driven by re-decoding a growing window between VAD boundaries.
    Batch,
}

/// Measured cost and quality. Populated by CI from `summo-bench`, refined per-machine by autotune.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub rss_mb: RssProfile,
    #[serde(default)]
    pub vram_mb: u32,
    /// Refuse to load below this much free RAM rather than OOM mid-meeting.
    #[serde(default)]
    pub min_ram_mb: u32,
    /// Real-time factor keyed by hardware class, e.g. `cpu_x86_avx512_8t`.
    #[serde(default)]
    pub rtf: BTreeMap<String, f32>,
    #[serde(default)]
    pub latency_ms: LatencyProfile,
    /// Quality metrics keyed by benchmark, e.g. `wer_fleurs_vi`.
    #[serde(default)]
    pub quality: BTreeMap<String, f32>,
    #[serde(default)]
    pub accel: Vec<Accel>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RssProfile {
    #[serde(default)]
    pub idle: u32,
    #[serde(default)]
    pub peak: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyProfile {
    /// Time from speech onset to the first partial reaching the UI.
    #[serde(default)]
    pub first_partial: u32,
    #[serde(default)]
    pub finalize_p50: u32,
    #[serde(default)]
    pub finalize_p95: u32,
}

/// Execution providers a model's files were exported for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accel {
    Cpu,
    Cuda,
    CoreMl,
    Metal,
    Vulkan,
    DirectMl,
}

/// One file belonging to a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Logical name referenced by `params`, e.g. `encoder.int8.onnx`.
    pub name: String,
    /// Lowercase hex sha256. Identity of the blob; a mismatch is always fatal.
    pub sha256: String,
    pub size: u64,
    /// Preferred source (CDN).
    pub url: String,
    /// Fallbacks tried in order — typically the upstream HuggingFace URL, so the model stays
    /// installable even if our CDN is gone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror: Vec<String>,
    /// Restrict this file to one platform, e.g. `linux-x64`, `macos-arm64`, `windows-x64`.
    ///
    /// Needed because some entries are native libraries or platform-specific ONNX exports. `None`
    /// means the file is portable and always fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

/// This build's platform tag, matching [`FileEntry::platform`].
#[must_use]
pub fn current_platform() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{}-{arch}", std::env::consts::OS)
}

/// A complete model description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub id: ModelId,
    pub name: String,
    pub task: Task,
    pub mode: Mode,
    /// Runtime that can execute this model, e.g. `sherpa-onnx/transducer-offline`.
    pub runtime: String,
    #[serde(default)]
    pub langs: Vec<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    /// SPDX identifier. Required — an unlicensed model cannot ship in a product we sell.
    pub license: String,
    /// Upstream credit. Required for attribution licenses such as CC-BY-4.0.
    #[serde(default)]
    pub attribution: Option<String>,
    /// Whether Summo may host and serve these files.
    ///
    /// `false` means the model's licence does not permit us to redistribute it — the user installs
    /// it from upstream themselves. Such manifests must not point at our CDN, which is checked in
    /// [`Manifest::validate`] so a mirroring job cannot quietly make us the distributor.
    #[serde(default = "default_true")]
    pub redistributable: bool,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub profile: Profile,
    pub files: Vec<FileEntry>,
    /// Runtime-specific knobs passed through to the engine.
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Manifest {
    /// Parse and validate. Validation is not optional: manifests arrive over the network and their
    /// contents become filesystem paths and HTTP requests.
    pub fn parse(json: &str) -> Result<Self> {
        let manifest: Self = serde_json::from_str(json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |reason: String| Error::InvalidManifest {
            id: self.id.to_string(),
            reason,
        };

        if self.schema != SCHEMA_VERSION {
            return Err(bad(format!(
                "schema {} unsupported (this build understands {SCHEMA_VERSION})",
                self.schema
            )));
        }
        if self.name.trim().is_empty() {
            return Err(bad("name is empty".into()));
        }
        if self.license.trim().is_empty() {
            return Err(bad("license is required".into()));
        }
        if self.runtime.trim().is_empty() {
            return Err(bad("runtime is required".into()));
        }
        if self.files.is_empty() {
            return Err(bad("no files listed".into()));
        }

        let mut seen = std::collections::HashSet::new();
        for f in &self.files {
            if f.name.trim().is_empty() {
                return Err(bad("file with empty name".into()));
            }
            // File names become path segments under the model directory.
            if f.name.contains("..") || f.name.contains('/') || f.name.contains('\\') {
                return Err(bad(format!(
                    "file name `{}` must not contain a path",
                    f.name
                )));
            }
            if !seen.insert(&f.name) {
                return Err(bad(format!("duplicate file name `{}`", f.name)));
            }
            if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(bad(format!("file `{}` has a malformed sha256", f.name)));
            }
            if f.sha256.bytes().any(|b| b.is_ascii_uppercase()) {
                return Err(bad(format!("file `{}` sha256 must be lowercase", f.name)));
            }
            if f.size == 0 {
                return Err(bad(format!("file `{}` has zero size", f.name)));
            }
            for url in std::iter::once(&f.url).chain(&f.mirror) {
                if !(url.starts_with("https://") || url.starts_with("file://")) {
                    return Err(bad(format!(
                        "file `{}` url `{url}` must be https:// or file://",
                        f.name
                    )));
                }
            }
        }

        // Attribution-required licenses are the ones most likely to be violated by accident.
        if self.license.starts_with("CC-BY") && self.attribution.is_none() {
            return Err(bad(format!(
                "license {} requires an `attribution` field",
                self.license
            )));
        }

        // A model we may not redistribute must be fetched from upstream. If it referenced our CDN,
        // mirroring it would make us the distributor and put us in breach of its licence.
        if !self.redistributable {
            for f in &self.files {
                for url in std::iter::once(&f.url).chain(&f.mirror) {
                    if url.contains("summo.app") {
                        return Err(bad(format!(
                            "`{}` is not redistributable but `{}` points at our own CDN",
                            f.name, url
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Files needed on this machine: portable ones plus those tagged for the current platform.
    pub fn platform_files(&self) -> impl Iterator<Item = &FileEntry> {
        let here = current_platform();
        self.files
            .iter()
            .filter(move |f| f.platform.as_ref().is_none_or(|p| *p == here))
    }

    /// Total bytes to fetch on this machine.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.platform_files().map(|f| f.size).sum()
    }

    pub fn file(&self, name: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.name == name)
    }

    /// A `params` entry that names a file, resolved to that file's entry.
    ///
    /// Runtimes are configured like `{"encoder": "encoder.int8.onnx"}`; this turns that indirection
    /// into something loadable.
    pub fn param_file(&self, key: &str) -> Option<&FileEntry> {
        self.params.get(key)?.as_str().and_then(|n| self.file(n))
    }

    /// Best-known RTF for a hardware class, falling back to the worst measured value so an unknown
    /// machine is assumed slow rather than fast.
    #[must_use]
    pub fn rtf_for(&self, hw_key: &str) -> Option<f32> {
        self.profile.rtf.get(hw_key).copied().or_else(|| {
            self.profile
                .rtf
                .values()
                .copied()
                .fold(None, |acc: Option<f32>, v| {
                    Some(acc.map_or(v, |a| a.max(v)))
                })
        })
    }

    /// Whether this model fits in the available RAM, with headroom for the rest of the app.
    #[must_use]
    pub fn fits_in_ram(&self, available_mb: u32) -> bool {
        let need = self.profile.min_ram_mb.max(self.profile.rss_mb.peak);
        need == 0 || available_mb >= need
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "id": "gipformer-65m",
            "name": "Gipformer 65M · Vietnamese",
            "task": "asr",
            "mode": "batch",
            "runtime": "sherpa-onnx/transducer-offline",
            "langs": ["vi"],
            "license": "Apache-2.0",
            "attribution": "k2-fsa/icefall",
            "profile": {
                "rss_mb": {"idle": 150, "peak": 800},
                "min_ram_mb": 1024,
                "rtf": {"cpu_x86_avx512_8t": 0.017, "cpu_arm_m1_4t": 0.03},
                "quality": {"wer_fleurs_vi": 0.024},
                "accel": ["cpu", "coreml"]
            },
            "files": [{
                "name": "encoder.int8.onnx",
                "sha256": "a".repeat(64),
                "size": 42_000_000u64,
                "url": "https://cdn.summo.app/blobs/sha256/aaa",
                "mirror": ["https://huggingface.co/k2-fsa/x/resolve/main/encoder.int8.onnx"]
            }],
            "params": {"encoder": "encoder.int8.onnx", "num_threads": "auto"}
        })
    }

    fn parse(v: serde_json::Value) -> Result<Manifest> {
        Manifest::parse(&v.to_string())
    }

    #[test]
    fn valid_manifest_round_trips() {
        let m = parse(base()).unwrap();
        assert_eq!(m.mode, Mode::Batch);
        assert_eq!(m.task, Task::Asr);
        assert_eq!(m.total_bytes(), 42_000_000);
        assert_eq!(m.param_file("encoder").unwrap().name, "encoder.int8.onnx");
        let back = Manifest::parse(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn future_schema_is_rejected_not_guessed() {
        let mut v = base();
        v["schema"] = serde_json::json!(99);
        assert!(parse(v).is_err());
    }

    #[test]
    fn file_names_cannot_escape_the_model_directory() {
        for name in ["../../etc/passwd", "sub/dir.onnx", "a\\b.onnx"] {
            let mut v = base();
            v["files"][0]["name"] = serde_json::json!(name);
            assert!(parse(v).is_err(), "`{name}` should be rejected");
        }
    }

    #[test]
    fn urls_must_be_https_or_file() {
        let mut v = base();
        v["files"][0]["url"] = serde_json::json!("http://cdn.summo.app/x");
        assert!(parse(v).is_err(), "plain http would allow silent tampering");
    }

    #[test]
    fn malformed_digests_are_rejected() {
        for digest in ["abc", &"z".repeat(64), &"A".repeat(64)] {
            let mut v = base();
            v["files"][0]["sha256"] = serde_json::json!(digest);
            assert!(parse(v).is_err(), "`{digest}` should be rejected");
        }
    }

    #[test]
    fn cc_by_without_attribution_fails_ci() {
        let mut v = base();
        v["license"] = serde_json::json!("CC-BY-4.0");
        v.as_object_mut().unwrap().remove("attribution");
        let err = parse(v).unwrap_err().to_string();
        assert!(err.contains("attribution"), "got: {err}");
    }

    #[test]
    fn missing_license_fails() {
        let mut v = base();
        v["license"] = serde_json::json!("");
        assert!(parse(v).is_err());
    }

    #[test]
    fn unknown_hardware_assumes_the_slowest_measured_rtf() {
        let m = parse(base()).unwrap();
        assert_eq!(m.rtf_for("cpu_x86_avx512_8t"), Some(0.017));
        assert_eq!(
            m.rtf_for("cpu_riscv_1t"),
            Some(0.03),
            "unknown hw must not look fast"
        );
    }

    #[test]
    fn ram_check_uses_the_larger_of_min_and_peak() {
        let m = parse(base()).unwrap();
        assert!(!m.fits_in_ram(512));
        assert!(m.fits_in_ram(2048));
    }

    #[test]
    fn platform_tagged_files_are_filtered_to_this_machine() {
        let mut v = base();
        let here = current_platform();
        v["files"] = serde_json::json!([
            {"name": "tokens.txt", "sha256": "a".repeat(64), "size": 100,
             "url": "https://cdn.summo.app/a"},
            {"name": "lib.here", "sha256": "b".repeat(64), "size": 200,
             "url": "https://cdn.summo.app/b", "platform": here},
            {"name": "lib.elsewhere", "sha256": "c".repeat(64), "size": 9999,
             "url": "https://cdn.summo.app/c", "platform": "plan9-vax"},
        ]);
        let m = parse(v).unwrap();
        let names: Vec<&str> = m.platform_files().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["tokens.txt", "lib.here"]);
        assert_eq!(
            m.total_bytes(),
            300,
            "other platforms must not count toward the download"
        );
    }

    #[test]
    fn models_default_to_redistributable() {
        assert!(parse(base()).unwrap().redistributable);
    }

    #[test]
    fn non_redistributable_models_may_not_point_at_our_cdn() {
        let mut v = base();
        v["redistributable"] = serde_json::json!(false);
        let err = parse(v.clone()).unwrap_err().to_string();
        assert!(err.contains("our own CDN"), "got: {err}");

        // Same model served from upstream is fine.
        v["files"][0]["url"] = serde_json::json!("https://github.com/ten-framework/ten-vad/x");
        v["files"][0]["mirror"] = serde_json::json!([]);
        assert!(parse(v).is_ok());
    }

    #[test]
    fn duplicate_file_names_are_rejected() {
        let mut v = base();
        let dup = v["files"][0].clone();
        v["files"].as_array_mut().unwrap().push(dup);
        assert!(parse(v).is_err());
    }
}
