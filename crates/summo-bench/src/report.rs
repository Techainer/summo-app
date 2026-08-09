//! Benchmark results and their rendering.

use serde::{Deserialize, Serialize};

/// Accuracy and cost for one VAD backend over one dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VadMetrics {
    /// Fraction of frames the detector called speech that really were.
    pub precision: f64,
    /// Fraction of real speech frames the detector found.
    pub recall: f64,
    pub f1: f64,
    /// Frames called speech during labelled silence, as a fraction of all silence frames. This is
    /// the "fan noise trips the detector" number.
    pub false_trigger_rate: f64,

    /// Median delay between real speech starting and the detector reacting, milliseconds.
    ///
    /// Largely hidden from the user by the gate's pre-roll, but a large value still costs audio
    /// context at the start of an utterance.
    pub onset_ms_p50: f64,
    /// Median delay between real speech *stopping* and the detector going quiet, milliseconds.
    ///
    /// This one is felt directly: the gate cannot close a segment until the detector releases, so
    /// it is added to the time before final text appears.
    pub release_ms_p50: f64,
    pub release_ms_p95: f64,

    /// Detector compute time divided by audio duration.
    pub rtf: f64,
    /// Total wall-clock spent in the detector, milliseconds.
    pub compute_ms: f64,
    pub audio_secs: f64,
    pub frames: usize,
}

/// One backend's run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VadReport {
    pub backend: String,
    /// Samples per detector call, so results at different hop sizes stay comparable.
    pub frame_len: usize,
    pub threshold: f32,
    pub dataset: String,
    pub clips: usize,
    pub metrics: VadMetrics,
    /// Licence note surfaced in the report, because a faster backend that cannot be shipped is not
    /// a win.
    pub license: String,
    /// Whether Summo may distribute this backend.
    pub redistributable: bool,
}

impl VadReport {
    /// Render a set of reports as a Markdown table for `docs/benchmarks.md`.
    #[must_use]
    pub fn to_markdown(reports: &[Self]) -> String {
        let mut out = String::new();
        out.push_str("| Backend | Frame | F1 | Precision | Recall | False trigger | Onset p50 | Release p50 | Release p95 | RTF | Licence | Shippable |\n");
        out.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|:-:|\n");
        for r in reports {
            let m = &r.metrics;
            out.push_str(&format!(
                "| {} | {} | {:.3} | {:.3} | {:.3} | {:.1}% | {:.0} ms | {:.0} ms | {:.0} ms | {:.4} | {} | {} |\n",
                r.backend,
                r.frame_len,
                m.f1,
                m.precision,
                m.recall,
                m.false_trigger_rate * 100.0,
                m.onset_ms_p50,
                m.release_ms_p50,
                m.release_ms_p95,
                m.rtf,
                r.license,
                if r.redistributable { "yes" } else { "**no**" },
            ));
        }
        out
    }
}

/// Percentile of an unsorted sample, using nearest-rank. Returns 0.0 for an empty sample.
#[must_use]
pub fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (p / 100.0 * values.len() as f64).ceil() as usize;
    values[rank.saturating_sub(1).min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_nearest_rank() {
        let mut v = vec![10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&mut v.clone(), 50.0), 20.0);
        assert_eq!(percentile(&mut v.clone(), 95.0), 40.0);
        assert_eq!(percentile(&mut v, 100.0), 40.0);
    }

    #[test]
    fn percentile_of_nothing_is_zero() {
        assert_eq!(percentile(&mut [], 50.0), 0.0);
    }

    #[test]
    fn markdown_marks_unshippable_backends() {
        let report = VadReport {
            backend: "some-vendor-vad".into(),
            frame_len: 256,
            threshold: 0.35,
            dataset: "testset".into(),
            clips: 30,
            metrics: VadMetrics {
                precision: 0.9,
                recall: 0.95,
                f1: 0.925,
                false_trigger_rate: 0.02,
                onset_ms_p50: 30.0,
                release_ms_p50: 90.0,
                release_ms_p95: 200.0,
                rtf: 0.01,
                compute_ms: 100.0,
                audio_secs: 262.0,
                frames: 16_000,
            },
            license: "Source-available, non-compete".into(),
            redistributable: false,
        };
        let md = VadReport::to_markdown(&[report]);
        assert!(
            md.contains("**no**"),
            "licence blockers must be visible in the table"
        );
        assert!(md.contains("some-vendor-vad"));
    }
}
