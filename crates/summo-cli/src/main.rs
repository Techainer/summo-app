//! `summo` — command line access to the model registry and the local store.
//!
//! Deliberately Ollama-shaped: `pull`, `list`, `rm`. The registry is a set of static JSON files, so
//! everything here works against a local directory as readily as against our CDN.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use summo_core::{ModelId, paths::Paths};
use summo_models::{Downloader, Manifest, ModelStore, Registry, RegistrySource, hw::HwProfile};

#[cfg(feature = "transcribe")]
mod transcribe;

#[derive(Parser)]
#[command(name = "summo", version, about)]
struct Cli {
    /// Data directory. Defaults to the platform application-data path, or `SUMMO_HOME`.
    #[arg(long, global = true)]
    home: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download a model and install it into the local blob store.
    Pull {
        /// Model id, e.g. `silero-vad-v5`.
        id: String,
        /// Registry to resolve from: an https URL or a directory. Defaults to the built-in chain.
        #[arg(long)]
        registry: Option<String>,
    },
    /// List installed models.
    List,
    /// Remove a model, reclaiming any blobs nothing else references.
    Rm { id: String },
    /// Delete blobs no installed model references.
    Gc,
    /// Show what this machine looks like to the model picker.
    Hw,
    /// Registry maintenance.
    #[command(subcommand)]
    Registry(RegistryCmd),

    /// Transcribe a 16 kHz mono WAV with the full live pipeline.
    #[cfg(feature = "transcribe")]
    Transcribe {
        audio: std::path::PathBuf,
        /// Directory holding the transducer model files.
        #[arg(long)]
        model_dir: std::path::PathBuf,
        /// Silero VAD ONNX file.
        #[arg(long)]
        vad: std::path::PathBuf,
        #[arg(long, default_value_t = 4)]
        threads: usize,
        /// How often the open utterance is re-decoded for partial text.
        #[arg(long, default_value_t = 150)]
        partial_step_ms: u32,
        /// Print partial text as it is produced.
        #[arg(long)]
        partials: bool,
    },
}

#[derive(Subcommand)]
enum RegistryCmd {
    /// Validate every manifest in a registry directory.
    ///
    /// Run in CI on the registry repository: it is what stops a manifest with a missing licence, a
    /// malformed digest, or a non-redistributable model pointed at our own CDN from being merged.
    Check { dir: std::path::PathBuf },
    /// List models available from a registry.
    Ls {
        #[arg(long)]
        registry: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let paths = match &cli.home {
        Some(dir) => Paths::at(dir),
        None => Paths::discover()?,
    };

    match cli.command {
        Command::Pull { id, registry } => pull(&paths, &id, registry.as_deref()).await,
        Command::List => list(&paths),
        Command::Rm { id } => remove(&paths, &id),
        Command::Gc => gc(&paths),
        Command::Hw => show_hw(),
        Command::Registry(RegistryCmd::Check { dir }) => check_registry(&dir),
        Command::Registry(RegistryCmd::Ls { registry }) => list_registry(registry.as_deref()).await,
        #[cfg(feature = "transcribe")]
        Command::Transcribe {
            audio,
            model_dir,
            vad,
            threads,
            partial_step_ms,
            partials,
        } => transcribe::run(&transcribe::Options {
            audio,
            model_dir,
            vad_model: vad,
            threads,
            partial_step_ms,
            show_partials: partials,
        }),
    }
}

fn registry_for(spec: Option<&str>) -> Result<Registry> {
    Ok(match spec {
        Some(s) => Registry::with_sources(vec![RegistrySource::parse(s)?])?,
        None => Registry::discover()?,
    })
}

async fn pull(paths: &Paths, id: &str, registry: Option<&str>) -> Result<()> {
    paths.ensure()?;
    let id = ModelId::parse(id).map_err(anyhow::Error::msg)?;

    let manifest = registry_for(registry)?.manifest(&id).await?;
    let store = ModelStore::new(paths.clone());

    if !manifest.redistributable {
        // Worth saying out loud: the user is fetching this from upstream under its own terms, and
        // Summo is not the distributor.
        tracing::warn!(
            license = %manifest.license,
            "not redistributed by Summo; fetched from upstream under its own licence"
        );
    }

    let hw = HwProfile::detect();
    if !manifest.fits_in_ram(hw.available_ram_mb) {
        tracing::warn!(
            needs_mb = manifest
                .profile
                .min_ram_mb
                .max(manifest.profile.rss_mb.peak),
            available_mb = hw.available_ram_mb,
            "this model may not fit in available memory"
        );
    }

    let downloader = Downloader::new(paths.downloads())?;
    let total = manifest.total_bytes();
    let mut last_pct = u8::MAX;

    let installed = store
        .install(&manifest, &downloader, |p| {
            if p.pct() != last_pct {
                last_pct = p.pct();
                tracing::info!(
                    pct = last_pct,
                    done = p.done,
                    total = p.total,
                    "downloading"
                );
            }
        })
        .await?;

    println!(
        "installed {} ({}) — {} file(s), {}",
        installed.manifest.id,
        installed.manifest.license,
        installed.files.len(),
        human_bytes(total)
    );
    Ok(())
}

fn list(paths: &Paths) -> Result<()> {
    let store = ModelStore::new(paths.clone());
    let models = store.list();
    if models.is_empty() {
        println!("no models installed. try `summo pull silero-vad-v5`");
        return Ok(());
    }
    println!(
        "{:<24} {:<8} {:<7} {:>10}  LICENCE",
        "ID", "TASK", "MODE", "SIZE"
    );
    for m in &models {
        println!(
            "{:<24} {:<8} {:<7} {:>10}  {}",
            m.id.as_str(),
            format!("{:?}", m.task).to_lowercase(),
            format!("{:?}", m.mode).to_lowercase(),
            human_bytes(m.total_bytes()),
            m.license
        );
    }
    println!("\n{} on disk", human_bytes(store.disk_usage()));
    Ok(())
}

fn remove(paths: &Paths, id: &str) -> Result<()> {
    let id = ModelId::parse(id).map_err(anyhow::Error::msg)?;
    let freed = ModelStore::new(paths.clone()).remove(&id)?;
    println!("removed {id}, freed {}", human_bytes(freed));
    Ok(())
}

fn gc(paths: &Paths) -> Result<()> {
    let freed = ModelStore::new(paths.clone()).gc()?;
    println!("reclaimed {}", human_bytes(freed));
    Ok(())
}

fn show_hw() -> Result<()> {
    let hw = HwProfile::detect();
    println!("{:<18} {}-{}", "platform", hw.os, hw.arch);
    println!("{:<18} {}", "cpu", hw.cpu_brand);
    println!(
        "{:<18} {} physical / {} logical",
        "cores", hw.cores, hw.logical_cpus
    );
    println!("{:<18} {}", "features", hw.features.tag());
    println!(
        "{:<18} {} MB total, {} MB available",
        "memory", hw.total_ram_mb, hw.available_ram_mb
    );
    println!("{:<18} {:?}", "accel", hw.accel);
    println!("{:<18} {}", "threads", hw.recommended_threads());
    println!("{:<18} {}", "benchmark key", hw.key());
    Ok(())
}

/// Validate every manifest in a registry directory.
fn check_registry(dir: &std::path::Path) -> Result<()> {
    let models_dir = dir.join("models");
    let entries = std::fs::read_dir(&models_dir)
        .with_context(|| format!("cannot read {}", models_dir.display()))?;

    let mut checked = 0;
    let mut failures = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;

        match Manifest::parse(&body) {
            Ok(m) => {
                // The file name is what a client requests, so it has to match the declared id.
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                if stem != m.id.as_str() {
                    failures.push(format!(
                        "{}: file name `{stem}` does not match id `{}`",
                        path.display(),
                        m.id
                    ));
                }
                checked += 1;
                println!(
                    "ok   {:<24} {:<42} {}",
                    m.id.as_str(),
                    m.license,
                    if m.redistributable {
                        ""
                    } else {
                        "(not redistributable)"
                    }
                );
            }
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }

    let index = dir.join("index.json");
    if index.is_file() {
        let body = std::fs::read_to_string(&index)?;
        if let Err(e) = serde_json::from_str::<serde_json::Value>(&body) {
            failures.push(format!("{}: {e}", index.display()));
        }
    } else {
        failures.push(format!("{} is missing", index.display()));
    }

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("FAIL {f}");
        }
        bail!("{} manifest(s) failed validation", failures.len());
    }
    println!("\n{checked} manifest(s) valid");
    Ok(())
}

async fn list_registry(spec: Option<&str>) -> Result<()> {
    let index = registry_for(spec)?.index().await?;
    println!(
        "{:<24} {:<8} {:<7} {:>10}  LICENCE",
        "ID", "TASK", "MODE", "SIZE"
    );
    for m in &index.models {
        println!(
            "{:<24} {:<8} {:<7} {:>10}  {}",
            m.id.as_str(),
            format!("{:?}", m.task).to_lowercase(),
            format!("{:?}", m.mode).to_lowercase(),
            human_bytes(m.size_bytes),
            m.license
        );
    }
    Ok(())
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
