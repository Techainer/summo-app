//! `summo-bench` — measure the things that decide Summo's defaults.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use summo_bench::{dataset::load_dataset, report::VadReport, vad::evaluate};

#[derive(Parser)]
#[command(name = "summo-bench", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate VAD backends on a labelled dataset.
    Vad {
        /// Directory of 16 kHz mono WAVs, each with a sibling `.scv` label file.
        #[arg(long)]
        dataset: std::path::PathBuf,

        /// Backend spec, repeatable. `silero:<path-to.onnx>` or `ten-vad:<hop>` (160 or 256).
        #[arg(long = "backend", required = true)]
        backends: Vec<String>,

        /// Decision threshold applied to the probability.
        #[arg(long, default_value_t = 0.35)]
        threshold: f32,

        /// Try a range of thresholds per backend and report each one's best-F1 operating point.
        ///
        /// Backends do not share a probability calibration, so a single fixed threshold flatters
        /// whichever one happens to be scaled to match it. Comparing each at its own best operating
        /// point is the only like-for-like reading.
        #[arg(long)]
        sweep: bool,

        /// Write JSON results here.
        #[arg(long)]
        json: Option<std::path::PathBuf>,

        /// Write a Markdown table here (for `docs/benchmarks.md`).
        #[arg(long)]
        markdown: Option<std::path::PathBuf>,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    match Cli::parse().command {
        Command::Vad {
            dataset,
            backends,
            threshold,
            sweep,
            json,
            markdown,
        } => run_vad(
            &dataset,
            &backends,
            threshold,
            sweep,
            json.as_deref(),
            markdown.as_deref(),
        ),
    }
}

/// Thresholds tried in sweep mode.
const SWEEP: &[f32] = &[
    0.15, 0.20, 0.25, 0.30, 0.35, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90,
];

fn run_vad(
    dataset: &std::path::Path,
    backends: &[String],
    threshold: f32,
    sweep: bool,
    json: Option<&std::path::Path>,
    markdown: Option<&std::path::Path>,
) -> Result<()> {
    let clips = load_dataset(dataset)?;
    let audio_secs: f64 = clips.iter().map(|c| c.duration()).sum();
    tracing::info!(
        clips = clips.len(),
        audio_secs = format!("{audio_secs:.1}"),
        "dataset loaded"
    );

    let mut reports = Vec::new();
    for spec in backends {
        let (mut vad, license, redistributable) = build_backend(spec)?;
        tracing::info!(backend = %spec, frame_len = vad.frame_len(), "evaluating");

        // In sweep mode keep the run with the highest F1; that is each backend's own best
        // operating point rather than one imposed on it.
        let mut best: Option<(f32, summo_bench::VadMetrics)> = None;
        for &t in if sweep {
            SWEEP
        } else {
            std::slice::from_ref(&threshold)
        } {
            let metrics = evaluate(vad.as_mut(), &clips, t)?;
            tracing::info!(
                threshold = t,
                f1 = format!("{:.3}", metrics.f1),
                precision = format!("{:.3}", metrics.precision),
                recall = format!("{:.3}", metrics.recall),
                release_p50_ms = format!("{:.0}", metrics.release_ms_p50),
                rtf = format!("{:.4}", metrics.rtf),
                "measured"
            );
            if best.as_ref().is_none_or(|(_, b)| metrics.f1 > b.f1) {
                best = Some((t, metrics));
            }
        }
        let (used_threshold, metrics) = best.expect("at least one threshold is always evaluated");

        reports.push(VadReport {
            backend: spec.clone(),
            frame_len: vad.frame_len(),
            threshold: used_threshold,
            dataset: dataset.display().to_string(),
            clips: clips.len(),
            metrics,
            license: license.to_string(),
            redistributable,
        });
    }

    let table = VadReport::to_markdown(&reports);
    println!("\n{table}");

    if let Some(path) = json {
        std::fs::write(path, serde_json::to_vec_pretty(&reports)?)
            .with_context(|| format!("cannot write {}", path.display()))?;
        tracing::info!(path = %path.display(), "wrote json");
    }
    if let Some(path) = markdown {
        std::fs::write(path, &table).with_context(|| format!("cannot write {}", path.display()))?;
        tracing::info!(path = %path.display(), "wrote markdown");
    }
    Ok(())
}

/// Construct a backend from a `name:arg` spec, along with its licence facts.
///
/// Licence is returned next to the metrics deliberately: a backend that wins on latency but cannot
/// legally ship is not a candidate, and burying that in a separate document is how it gets missed.
#[allow(unused_variables)]
fn build_backend(spec: &str) -> Result<(Box<dyn summo_vad::Vad>, &'static str, bool)> {
    let (name, arg) = spec.split_once(':').unwrap_or((spec, ""));
    match name {
        #[cfg(feature = "silero")]
        "silero" => {
            if arg.is_empty() {
                bail!("silero backend needs a model path: --backend silero:/path/to/silero.onnx");
            }
            let vad = summo_vad::silero::SileroVad::load(arg, 1)?;
            Ok((Box::new(vad), "MIT", true))
        }
        #[cfg(feature = "ten-vad")]
        "ten-vad" => {
            let hop: usize = if arg.is_empty() {
                summo_vad::ten::HOP_16MS
            } else {
                arg.parse().context("ten-vad hop size must be 160 or 256")?
            };
            let vad = summo_vad::ten::TenVad::new(hop)?;
            Ok((Box::new(vad), "Apache-2.0 + extra conditions", false))
        }
        other => bail!(
            "unknown or disabled backend `{other}`. Enable the matching cargo feature \
             (--features silero,ten-vad)."
        ),
    }
}
