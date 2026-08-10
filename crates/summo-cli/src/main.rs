//! `summo` — command line access to the model registry and the local store.
//!
//! Deliberately Ollama-shaped: `pull`, `list`, `rm`. The registry is a set of static JSON files, so
//! everything here works against a local directory as readily as against our CDN.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use summo_core::{ModelId, paths::Paths};
use summo_models::{Downloader, Manifest, ModelStore, Registry, RegistrySource, hw::HwProfile};

mod ai;
mod library;

#[cfg(feature = "transcribe")]
mod transcribe;

#[derive(Parser)]
#[command(name = "summo", version, about)]
struct Cli {
    /// Data directory. Defaults to `~/.summo`, or `SUMMO_HOME`.
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

    /// Rank the models available for a language on this machine, and say why.
    Recommend {
        /// ISO language code, e.g. `vi` or `en`.
        #[arg(long, default_value = "vi")]
        lang: String,
        /// Include models from the registry, not only the installed ones.
        #[arg(long)]
        registry: Option<String>,
    },

    /// Pick the right models for this machine and language, install them, and be ready to record.
    ///
    /// The one-command path: measure the machine, rank what is available, download what is missing,
    /// and print the session to start with.
    Setup {
        #[arg(long, default_value = "vi")]
        lang: String,
        #[arg(long)]
        registry: Option<String>,
        /// Print the plan without downloading anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Registry maintenance.
    #[command(subcommand)]
    Registry(RegistryCmd),

    /// Summarise a meeting and write the summary into its own file.
    Summarize {
        /// Path to a meeting Markdown file.
        meeting: std::path::PathBuf,
        /// `brief`, `standard` or `detailed`.
        #[arg(long, default_value = "standard")]
        style: String,
        /// Language to write the summary in.
        #[arg(long, default_value = "Vietnamese")]
        language: String,
        /// Print without modifying the file.
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        provider: ai::ProviderArgs,
    },

    /// Ask a question about the meetings on disk.
    Ask {
        question: String,
        #[arg(long, default_value = "Vietnamese")]
        language: String,
        /// Excerpts to give the model.
        #[arg(long, default_value_t = 40)]
        limit: usize,
        #[command(flatten)]
        provider: ai::ProviderArgs,
    },

    /// Browse and organise the meetings on disk.
    #[command(subcommand, alias = "vault")]
    Meetings(library::MeetingCmd),

    /// Convert a meeting to another format.
    Export {
        meeting: std::path::PathBuf,
        /// `md`, `txt`, `srt`, `vtt`, `json` or `csv`.
        #[arg(long, default_value = "txt")]
        format: String,
        /// Write here instead of standard output.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Drop timestamps and join each speaker's consecutive lines.
        #[arg(long)]
        readable: bool,
    },

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
        /// Which runtime loads the model directory.
        #[arg(long, value_enum, default_value_t = transcribe::Engine::Transducer)]
        engine: transcribe::Engine,
        /// ISO language code for Whisper, e.g. `en` or `vi`. Omit to let it detect.
        #[arg(long)]
        lang: Option<String>,
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
        Command::Recommend { lang, registry } => {
            show_recommendation(&paths, &lang, registry.as_deref()).await
        }
        Command::Setup {
            lang,
            registry,
            dry_run,
        } => setup(&paths, &lang, registry.as_deref(), dry_run).await,
        Command::Registry(RegistryCmd::Check { dir }) => check_registry(&dir),
        Command::Registry(RegistryCmd::Ls { registry }) => list_registry(registry.as_deref()).await,
        Command::Summarize {
            meeting,
            style,
            language,
            dry_run,
            provider,
        } => {
            let style = match style.as_str() {
                "brief" => summo_llm::SummaryStyle::Brief,
                "standard" => summo_llm::SummaryStyle::Standard,
                "detailed" => summo_llm::SummaryStyle::Detailed,
                other => bail!("unknown style `{other}`. Use brief, standard or detailed."),
            };
            ai::summarize(&meeting, style, &language, &provider, !dry_run).await
        }
        Command::Ask {
            question,
            language,
            limit,
            provider,
        } => ai::ask(&paths, &question, &language, limit, &provider).await,
        Command::Meetings(cmd) => library::run(&paths, cmd),
        Command::Export {
            meeting,
            format,
            out,
            readable,
        } => export(&meeting, &format, out.as_deref(), readable),
        #[cfg(feature = "transcribe")]
        Command::Transcribe {
            audio,
            model_dir,
            vad,
            engine,
            lang,
            threads,
            partial_step_ms,
            partials,
        } => transcribe::run(&transcribe::Options {
            audio,
            model_dir,
            vad_model: vad,
            engine,
            language: lang,
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

/// Gather every manifest worth considering: installed ones plus, optionally, the registry.
async fn candidates(paths: &Paths, registry: Option<&str>) -> Result<Vec<summo_models::Manifest>> {
    let store = ModelStore::new(paths.clone());
    let mut manifests = store.list();

    if let Some(spec) = registry {
        let reg = registry_for(Some(spec))?;
        match reg.index().await {
            Ok(index) => {
                for entry in index.models {
                    if manifests.iter().any(|m| m.id == entry.id) {
                        continue;
                    }
                    match reg.manifest(&entry.id).await {
                        Ok(m) => manifests.push(m),
                        Err(e) => tracing::warn!(id = %entry.id, error = %e, "skipping"),
                    }
                }
            }
            // A registry that cannot be reached should not stop the app ranking what is already
            // installed — offline is a supported state, not an error.
            Err(e) => {
                tracing::warn!(error = %e, "registry unavailable; ranking installed models only")
            }
        }
    }
    Ok(manifests)
}

async fn show_recommendation(paths: &Paths, lang: &str, registry: Option<&str>) -> Result<()> {
    let hw = HwProfile::detect();
    let manifests = candidates(paths, registry).await?;
    let out = summo_models::recommend(&manifests, &hw, lang);

    println!(
        "machine  {} · {} cores · {} MB free\n",
        hw.key(),
        hw.cores,
        hw.available_ram_mb
    );

    if out.ranked.is_empty() {
        println!("No model can run {lang} on this machine.");
    }
    for (rank, scored) in out.ranked.iter().enumerate() {
        let marker = if scored.live_capable { "live" } else { "batch" };
        println!(
            "{}. {:<24} [{marker}]  {}",
            rank + 1,
            scored.id,
            scored.reason
        );
    }

    if !out.rejected.is_empty() {
        println!("\nNot offered:");
        for rejected in &out.rejected {
            println!("   {:<24} {}", rejected.id, rejected.reason);
        }
    }

    let (live, refine) = out.pair();
    if let Some(live) = live {
        println!("\nrecommended live model    {}", live.id);
        match refine {
            Some(refine) => println!("recommended refine model  {}", refine.id),
            None => println!("recommended refine model  none — nothing available is more accurate"),
        }
    }
    Ok(())
}

async fn setup(paths: &Paths, lang: &str, registry: Option<&str>, dry_run: bool) -> Result<()> {
    paths.ensure()?;
    let hw = HwProfile::detect();
    let store = ModelStore::new(paths.clone());

    println!(
        "machine  {} · {} cores · {} MB free",
        hw.key(),
        hw.cores,
        hw.available_ram_mb
    );

    let manifests = candidates(paths, registry).await?;
    let out = summo_models::recommend(&manifests, &hw, lang);
    let (live, refine) = out.pair();

    let Some(live) = live else {
        bail!(
            "no model available can run {lang} on this machine. \
             Point --registry at a registry with more models, or free up memory."
        );
    };

    // A voice detector is needed regardless of which speech model is chosen; without one there are
    // no utterance boundaries and nothing to decode.
    let vad = manifests
        .iter()
        .find(|m| m.task == summo_models::Task::Vad)
        .map(|m| m.id.to_string());

    let mut plan: Vec<String> = Vec::new();
    if let Some(vad) = &vad {
        plan.push(vad.clone());
    }
    plan.push(live.id.clone());
    if let Some(refine) = refine {
        plan.push(refine.id.clone());
    }

    println!("\nplan for {lang}:");
    for id in &plan {
        let installed = summo_core::ModelId::parse(id.as_str())
            .ok()
            .is_some_and(|parsed| store.installed(&parsed).is_ok());
        println!("  {} {id}", if installed { "have" } else { "pull" });
    }

    if dry_run {
        println!("\ndry run: nothing downloaded");
        return Ok(());
    }

    for id in &plan {
        let parsed = summo_core::ModelId::parse(id.as_str()).map_err(anyhow::Error::msg)?;
        if store.installed(&parsed).is_ok() {
            continue;
        }
        println!("\npulling {id}…");
        pull(paths, id, registry).await?;
    }

    if vad.is_none() {
        println!(
            "\nWarning: no voice detector is available. Recording will not start without one — \
             try `summo pull silero-vad-v5`."
        );
    }

    println!("\nready. Start a session with:");
    match refine {
        Some(refine) => println!("  live model {} refined by {}", live.id, refine.id),
        None => println!("  live model {}", live.id),
    }
    Ok(())
}

/// Render a meeting in another format.
fn export(
    meeting: &std::path::Path,
    format: &str,
    out: Option<&std::path::Path>,
    readable: bool,
) -> Result<()> {
    use summo_vault::{
        MeetingDoc,
        export::{Format, Options},
    };

    let format = Format::parse(format).with_context(|| {
        format!("unknown format `{format}`. Use md, txt, srt, vtt, json or csv.")
    })?;
    let body = std::fs::read_to_string(meeting)
        .with_context(|| format!("cannot read {}", meeting.display()))?;
    let doc = MeetingDoc::parse(&body)?;

    let options = if readable {
        Options::readable()
    } else {
        Options::default()
    };
    let rendered = summo_vault::export(&doc, format, options)?;

    match out {
        Some(path) => {
            std::fs::write(path, &rendered)
                .with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{rendered}"),
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
