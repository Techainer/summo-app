//! Hardware probe.
//!
//! Answers two questions the rest of the app keeps asking: *which execution provider should we
//! use*, and *which measured RTF row applies to this machine*. The second is why [`HwProfile::key`]
//! exists — benchmark numbers are meaningless unless they can be matched to comparable hardware.

use serde::{Deserialize, Serialize};

pub use crate::manifest::Accel;

/// CPU instruction-set features that materially change inference speed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuFeatures {
    pub avx2: bool,
    pub avx512: bool,
    /// AVX-512 VNNI — roughly doubles INT8 throughput where present.
    pub vnni: bool,
    /// ARM NEON, assumed present on every aarch64 target we ship.
    pub neon: bool,
}

impl CpuFeatures {
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            let cpuid = raw_cpuid::CpuId::new();
            let ext = cpuid.get_extended_feature_info();
            let avx2 = ext
                .as_ref()
                .is_some_and(raw_cpuid::ExtendedFeatures::has_avx2);
            let avx512 = ext
                .as_ref()
                .is_some_and(raw_cpuid::ExtendedFeatures::has_avx512f);
            let vnni = ext
                .as_ref()
                .is_some_and(raw_cpuid::ExtendedFeatures::has_avx512vnni);
            return Self {
                avx2,
                avx512,
                vnni,
                neon: false,
            };
        }
        #[cfg(target_arch = "aarch64")]
        {
            return Self {
                avx2: false,
                avx512: false,
                vnni: false,
                neon: true,
            };
        }
        #[allow(unreachable_code)]
        Self::default()
    }

    /// Short tag used in benchmark keys.
    #[must_use]
    pub fn tag(self) -> &'static str {
        if self.vnni {
            "x86_avx512vnni"
        } else if self.avx512 {
            "x86_avx512"
        } else if self.avx2 {
            "x86_avx2"
        } else if self.neon {
            "arm_neon"
        } else {
            "generic"
        }
    }
}

/// A snapshot of the machine, cached in `~/.summo/hw.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HwProfile {
    pub os: String,
    pub arch: String,
    /// Physical cores where the OS reports them, else logical.
    pub cores: usize,
    pub logical_cpus: usize,
    pub total_ram_mb: u32,
    pub available_ram_mb: u32,
    pub cpu_brand: String,
    pub features: CpuFeatures,
    /// Execution providers in preference order, best first.
    pub accel: Vec<Accel>,
}

impl HwProfile {
    #[must_use]
    pub fn detect() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());

        let logical_cpus = num_cpus::get();
        let cores = num_cpus::get_physical().max(1);
        let cpu_brand = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "unknown".into());

        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cores,
            logical_cpus,
            total_ram_mb: bytes_to_mb(sys.total_memory()),
            available_ram_mb: bytes_to_mb(sys.available_memory()),
            cpu_brand,
            features: CpuFeatures::detect(),
            accel: detect_accel(),
        }
    }

    /// Refresh only the volatile part. Cheap enough to call before each model load.
    pub fn refresh_memory(&mut self) {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        self.total_ram_mb = bytes_to_mb(sys.total_memory());
        self.available_ram_mb = bytes_to_mb(sys.available_memory());
    }

    /// Benchmark key for this machine, e.g. `cpu_x86_avx512vnni_8t`.
    ///
    /// Thread count is bucketed rather than exact so a 6-core and an 8-core laptop share a row
    /// instead of fragmenting the benchmark table into single-sample noise.
    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "cpu_{}_{}t",
            self.features.tag(),
            bucket_threads(self.recommended_threads())
        )
    }

    /// Threads to hand an inference runtime.
    ///
    /// Capped at 8 because the prototype measured scaling plateau past that on Cascade Lake, and
    /// because a meeting recorder that eats every core is a meeting recorder people uninstall.
    #[must_use]
    pub fn recommended_threads(&self) -> usize {
        self.cores.clamp(1, 8)
    }

    /// Best available execution provider.
    #[must_use]
    pub fn best_accel(&self) -> Accel {
        self.accel.first().copied().unwrap_or(Accel::Cpu)
    }

    /// Whether a model's declared accel list overlaps what this machine offers.
    #[must_use]
    pub fn supports(&self, model_accel: &[Accel]) -> bool {
        model_accel.is_empty() || model_accel.iter().any(|a| self.accel.contains(a))
    }
}

fn bytes_to_mb(bytes: u64) -> u32 {
    u32::try_from(bytes / (1024 * 1024)).unwrap_or(u32::MAX)
}

fn bucket_threads(threads: usize) -> usize {
    match threads {
        0..=2 => 2,
        3..=5 => 4,
        6..=11 => 8,
        _ => 16,
    }
}

/// Execution providers in preference order.
///
/// Detection is intentionally conservative — presence of a driver library, not a successful
/// allocation. The engine falls back to CPU if a provider fails to initialise at load time, which
/// is the only place the truth is really knowable.
fn detect_accel() -> Vec<Accel> {
    let mut out = Vec::new();

    #[cfg(target_os = "macos")]
    {
        // CoreML and Metal ship with the OS on every Mac we support.
        out.push(Accel::CoreMl);
        out.push(Accel::Metal);
    }

    #[cfg(not(target_os = "macos"))]
    {
        if has_cuda() {
            out.push(Accel::Cuda);
        }
        #[cfg(target_os = "windows")]
        out.push(Accel::DirectMl);
    }

    out.push(Accel::Cpu);
    out
}

#[cfg(not(target_os = "macos"))]
fn has_cuda() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/dev/nvidiactl").exists()
            || std::path::Path::new("/proc/driver/nvidia/version").exists()
    }
    #[cfg(target_os = "windows")]
    {
        std::path::Path::new(r"C:\Windows\System32\nvcuda.dll").exists()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_reports_something_plausible() {
        let hw = HwProfile::detect();
        assert!(hw.cores >= 1);
        assert!(hw.logical_cpus >= hw.cores.min(hw.logical_cpus));
        assert!(hw.total_ram_mb > 0);
        assert!(
            hw.accel.contains(&Accel::Cpu),
            "cpu must always be a fallback"
        );
        assert_eq!(hw.best_accel(), hw.accel[0]);
    }

    #[test]
    fn threads_are_capped_so_the_app_stays_polite() {
        let mut hw = HwProfile::detect();
        hw.cores = 64;
        assert_eq!(hw.recommended_threads(), 8);
        hw.cores = 1;
        assert_eq!(hw.recommended_threads(), 1);
    }

    #[test]
    fn bench_key_buckets_thread_counts() {
        assert_eq!(bucket_threads(1), 2);
        assert_eq!(bucket_threads(4), 4);
        assert_eq!(bucket_threads(8), 8);
        assert_eq!(bucket_threads(64), 16);
    }

    #[test]
    fn key_is_stable_and_prefixed() {
        let hw = HwProfile::detect();
        assert!(hw.key().starts_with("cpu_"), "got {}", hw.key());
        assert_eq!(hw.key(), HwProfile::detect().key());
    }

    #[test]
    fn empty_model_accel_means_runs_anywhere() {
        let hw = HwProfile::detect();
        assert!(hw.supports(&[]));
        assert!(hw.supports(&[Accel::Cpu]));
    }

    #[test]
    fn profile_round_trips_through_json() {
        let hw = HwProfile::detect();
        let back: HwProfile = serde_json::from_str(&serde_json::to_string(&hw).unwrap()).unwrap();
        assert_eq!(hw, back);
    }
}
