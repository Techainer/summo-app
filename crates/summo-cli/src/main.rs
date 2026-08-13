//! `summo` — command line access to the model registry and the local store.
//!
//! Deliberately Ollama-shaped: `pull`, `list`, `rm`. The registry is a set of static JSON files, so
//! everything here works against a local directory as readily as against our CDN.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use summo_core::{ModelId, paths::Paths};
use summo_models::{Downloader, Manifest, ModelStore, Registry, RegistrySource, hw::HwProfile};

mod ai;
#[cfg(feature = "dub")]
mod dub;
mod importer;
mod library;
mod sync;

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

    /// Turn recordings you already have into meetings.
    ///
    /// Takes a file or a folder. Video is fine — the audio is extracted and the video left alone.
    Import {
        /// A media file, or a folder of them.
        path: std::path::PathBuf,
        /// Report what would be imported without queueing anything.
        #[arg(long)]
        dry_run: bool,
        /// ISO language code for the recogniser. Omit to let the model detect.
        #[arg(long)]
        lang: Option<String>,
        /// Queue the files and exit instead of waiting for them.
        #[arg(long)]
        detach: bool,
    },

    /// Run Summo: the daemon and the interface, in this process.
    ///
    /// One command and one binary, the way `ollama serve` is. Nothing to install beside it, no
    /// second process to start, no directory of static files to keep in step.
    #[cfg(feature = "serve")]
    Serve {
        /// Port to listen on. `0` lets the OS pick, which is what avoids a collision on a machine
        /// already running something on 8710.
        #[arg(long, default_value_t = 8710)]
        port: u16,
        /// Print the address instead of opening a browser.
        #[arg(long)]
        no_open: bool,
        /// Allow pages served from this machine to reach the daemon.
        ///
        /// For developing the interface with Vite. Off by default, because the check it removes is
        /// the one that stops a web page reaching the microphone.
        #[arg(long)]
        dev: bool,
    },

    /// Serve the vault to an MCP client — Claude Code, Cursor — over stdio.
    ///
    /// Read-only. No tool here creates a task, edits a note or starts a recording: an MCP client is
    /// a model holding a tool list, and one that misreads an instruction should not be able to
    /// rewrite somebody's meeting notes.
    #[cfg(feature = "mcp")]
    Mcp,

    /// Speak a meeting's translation over its own recording.
    ///
    /// Needs a translation on disk — `summo translate` first — and a VITS voice directory.
    #[cfg(feature = "dub")]
    Dub {
        /// Meeting id.
        meeting: String,
        /// Language to speak, as it was translated.
        #[arg(long)]
        lang: String,
        /// Directory holding the voice's model files.
        #[arg(long)]
        voice: std::path::PathBuf,
        #[arg(long, default_value = "dub.wav")]
        out: std::path::PathBuf,
        /// Gain for the original underneath. 0 removes it.
        #[arg(long, default_value_t = 0.18)]
        under: f32,
        #[arg(long, default_value_t = 4)]
        threads: usize,
    },

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

    /// Keep this vault in step with a folder — a NAS mount, a synced drive, a USB stick.
    ///
    /// Everything is encrypted before it leaves, so whatever holds the folder cannot read it.
    Sync {
        /// The folder to sync through.
        to: std::path::PathBuf,
        /// A name for this machine, used in the name of any conflict copy it produces.
        #[arg(long)]
        machine: Option<String>,
        /// Show what would happen and change nothing.
        #[arg(long)]
        dry_run: bool,
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
        /// Run the same models through a pipeline chain instead of the hand-written loop.
        #[arg(long)]
        pipeline: bool,
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
    /// Generate a Markdown page per model, Ollama-style.
    ///
    /// Everything on a page is derived from its manifest, so the pages cannot drift from the models
    /// they describe. Run in CI on the registry repository after `check`.
    Pages {
        /// Registry directory, containing `models/*.json`.
        dir: std::path::PathBuf,
        /// Where to write the pages. Defaults to `<dir>/pages`.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Print what would be written without writing it.
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // stderr, always.
    //
    // Most commands print results to stdout and could log there too. `summo mcp` cannot: its stdout
    // is a JSON-RPC stream, one object per line, and a log line in it is a parse error the client
    // reports as the server being broken. Rather than making the destination depend on the
    // subcommand — which is the kind of rule that gets forgotten when the next one is added —
    // logging goes to stderr for everything. Nothing is lost: a terminal shows both.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    let paths = match &cli.home {
        Some(dir) => Paths::at(dir),
        None => Paths::discover()?,
    };

    match cli.command {
        Command::Pull { id, registry } => pull(&paths, &id, registry.as_deref()).await,
        Command::List => list(&paths),
        Command::Rm { id } => remove(&paths, &id),
        Command::Gc => gc(&paths),
        Command::Import {
            path,
            dry_run,
            lang,
            detach,
        } => import(&paths, &path, dry_run, lang.as_deref(), detach).await,
        #[cfg(feature = "serve")]
        Command::Serve { port, no_open, dev } => serve(&paths, port, no_open, dev).await,
        #[cfg(feature = "mcp")]
        Command::Mcp => mcp(&paths),
        #[cfg(feature = "dub")]
        Command::Dub {
            meeting,
            lang,
            voice,
            out,
            under,
            threads,
        } => dub::run(
            &paths,
            &dub::Options {
                meeting,
                lang,
                voice,
                out,
                under,
                threads,
            },
        ),
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
        Command::Registry(RegistryCmd::Pages { dir, out, dry_run }) => {
            registry_pages(&dir, out.as_deref(), dry_run)
        }
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
        Command::Sync {
            to,
            machine,
            dry_run,
        } => sync::run(&paths, &to, machine.as_deref(), dry_run),
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
            pipeline,
        } => transcribe::run(&transcribe::Options {
            audio,
            model_dir,
            vad_model: vad,
            engine,
            language: lang,
            threads,
            partial_step_ms,
            show_partials: partials,
            pipeline,
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

    // Which build fits this machine — accelerator, precision, free memory. Printed rather than
    // applied silently: a user who expected the fp32 export and got int8 should be able to see
    // that, and why, without reading the source.
    let choice = summo_models::variant::choose(&manifest, &hw);
    if let Some(name) = &choice.variant {
        println!("build     {name} ({})", choice.reason);
    }
    if !choice.alternatives.is_empty() {
        println!("  also ok {}", choice.alternatives.join(", "));
    }
    for rejected in &choice.rejected {
        println!("  skipped {} — {}", rejected.variant, rejected.why);
    }
    let manifest = manifest.with_variant(choice.variant.clone());

    let downloader = Downloader::new(paths.downloads())?;
    let total = manifest.total_bytes();
    let mut last_pct = u8::MAX;

    let installed = store
        .install_variant(&manifest, &downloader, choice.variant, |p| {
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
            summo_models::page::task_name(m.task),
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
    let mut published = Vec::new();
    // Manifests that exist but did not parse. Without this they look *absent* to the index check
    // below, which then reports "there is no models/x.json behind it" about a file sitting right
    // there — two failures for one cause, and the second one sends the reader to the wrong place.
    let mut unreadable = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;

        match Manifest::parse(&body) {
            Ok(m) => {
                published.push(m.id.to_string());
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
            Err(e) => {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    unreadable.push(stem.to_string());
                }
                failures.push(format!("{}: {e}", path.display()));
            }
        }
    }

    let index = dir.join("index.json");
    if index.is_file() {
        let body = std::fs::read_to_string(&index)?;
        match serde_json::from_str::<summo_models::registry::Index>(&body) {
            Ok(parsed) => failures.extend(index_disagreements(&parsed, &published, &unreadable)),
            Err(e) => failures.push(format!("{}: {e}", index.display())),
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

/// Ways `index.json` and `models/` can disagree, in the words of what it costs.
///
/// The index is what every client reads first: `summo ls`, the setup screen and the model picker
/// all render from it and never open a manifest until something is chosen. So a model published to
/// `models/` and forgotten in the index is not a cosmetic inconsistency — it is a model nobody can
/// install, and the only symptom is that it is not there, which is indistinguishable from it never
/// having been added. This check used to be `serde_json::from_str::<Value>`, which confirmed the
/// file was JSON and nothing else.
///
/// The reverse — an index row with no manifest behind it — is worse, because it is visible: the
/// model appears in the picker, the user chooses it, and the download 404s.
fn index_disagreements(
    index: &summo_models::registry::Index,
    published: &[String],
    unreadable: &[String],
) -> Vec<String> {
    let listed: Vec<String> = index.models.iter().map(|m| m.id.to_string()).collect();
    let mut out = Vec::new();

    for id in published {
        if !listed.contains(id) {
            out.push(format!(
                "index.json does not list `{id}`, so nothing can install it"
            ));
        }
    }
    for id in &listed {
        // A manifest that failed to parse has already been reported by name; saying it is also
        // missing would be a second, wrong explanation of the same problem.
        if !published.contains(id) && !unreadable.contains(id) {
            out.push(format!(
                "index.json lists `{id}` but there is no models/{id}.json behind it"
            ));
        }
    }
    out
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
            summo_models::page::task_name(m.task),
            format!("{:?}", m.mode).to_lowercase(),
            human_bytes(m.size_bytes),
            m.license
        );
    }
    Ok(())
}

/// Gather every manifest worth considering: what is installed, plus the catalogue.
///
/// The catalogue is consulted whether or not `--registry` was given. It used to be consulted *only*
/// when it was given, which meant `summo setup` on a machine with nothing installed ranked an empty
/// list and said "no model available can run vi on this machine. Point --registry at a registry
/// with more models, or free up memory" — on a machine with 200 GB free, about a model that
/// `summo pull` could fetch a second later from the default registry it had just declined to read.
///
/// That is the first command a person runs after downloading a release, so the failure was
/// everybody's, on the way in, and the message sent them looking at their memory.
async fn candidates(paths: &Paths, registry: Option<&str>) -> Result<Vec<summo_models::Manifest>> {
    let store = ModelStore::new(paths.clone());
    let mut manifests = store.list();

    {
        let reg = registry_for(registry)?;
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
    use summo_vault::export::{Format, Options};

    let format = Format::parse(format).with_context(|| {
        format!("unknown format `{format}`. Use md, txt, srt, vtt, json or csv.")
    })?;
    // Exporting takes a path to any file, which may be outside a vault entirely. Its own parent
    // stands in as the root: the id only matters for a file that has no frontmatter, and nothing
    // downstream of an export writes that id back.
    let root = meeting.parent().unwrap_or(std::path::Path::new("."));
    let doc = summo_vault::open(root, meeting)?;

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

/// A duration a person reads. Seconds under a minute, because "0 phút" for a voice memo reads as
/// a bug rather than as a short file.
fn length_of(seconds: f64) -> String {
    if seconds < 60.0 {
        return format!("{} giây", seconds.round() as u64);
    }
    let minutes = (seconds / 60.0).round() as u64;
    if minutes < 60 {
        return format!("{minutes} phút");
    }
    format!("{} giờ {} phút", minutes / 60, minutes % 60)
}

/// Turn recordings the user already has into meetings.
///
/// The work happens in the daemon: it owns the models, and a second implementation here would drift
/// from the one the app uses. This queues the files and follows them.
///
/// Importing a folder of forty files must not stop on the one that is corrupt, so each file is
/// reported on its own line and a failure moves to the next.
async fn import(
    paths: &Paths,
    path: &std::path::Path,
    dry_run: bool,
    lang: Option<&str>,
    detach: bool,
) -> Result<()> {
    let files = importer::targets(path)?;
    if files.is_empty() {
        println!("Không có file nào để nhập trong {}", path.display());
        return Ok(());
    }

    if dry_run {
        // ffmpeg is probed here and only here on this path: a dry run should still fail loudly if
        // the thing that does the decoding is missing.
        let tools = summo_media::probe()?;
        for file in &files {
            let title = summo_media::title_from(file);
            match tools.info(file) {
                Ok(info) if !info.has_audio => println!("  bỏ qua  {title} — không có âm thanh"),
                Ok(info) => println!("  sẽ nhập {title} — {}", length_of(info.duration_s)),
                Err(e) => println!("  lỗi     {title} — {e}"),
            }
        }
        println!("\n{} file (chạy thử)", files.len());
        return Ok(());
    }

    let handshake = importer::handshake(paths)?;
    let client = reqwest::Client::new();

    let mut jobs = Vec::new();
    for file in &files {
        let title = summo_media::title_from(file);
        match importer::start(&client, &handshake, file, lang).await {
            Ok(job) => {
                println!("  xếp hàng {title}");
                jobs.push(job);
            }
            Err(e) => println!("  lỗi     {title} — {e}"),
        }
    }

    if jobs.is_empty() {
        bail!("không nhập được file nào");
    }
    if detach {
        println!("\n{} file đang chạy trong nền.", jobs.len());
        return Ok(());
    }

    println!();
    let mut failed = 0usize;
    for job in &jobs {
        let mut current = importer::poll(&client, &handshake, &job.id).await?;
        while !current.finished() {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            current = importer::poll(&client, &handshake, &job.id).await?;
        }
        if current.state == "failed" {
            failed += 1;
        }
        println!("  {} — {}", current.title, current.line());
    }

    println!("\n{} xong, {failed} lỗi", jobs.len() - failed);
    Ok(())
}

/// Write one Markdown page per model, plus an index.
///
/// The upstream README, when the registry has one at `models/<id>.md`, is copied in verbatim rather
/// than summarised: a paraphrase of somebody else's documentation is how a licence notice quietly
/// loses the sentence that mattered.
fn registry_pages(
    dir: &std::path::Path,
    out: Option<&std::path::Path>,
    dry_run: bool,
) -> Result<()> {
    let models_dir = dir.join("models");
    let out_dir = out.map_or_else(|| dir.join("pages"), std::path::Path::to_path_buf);

    let entries = std::fs::read_dir(&models_dir)
        .with_context(|| format!("cannot read {}", models_dir.display()))?;

    let mut manifests = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        // A malformed manifest is `registry check`'s job to report. Failing here too would mean two
        // commands blaming the same file with different wording.
        match Manifest::parse(&body) {
            Ok(m) => manifests.push(m),
            Err(e) => bail!("{}: {e} — run `summo registry check` first", path.display()),
        }
    }

    if manifests.is_empty() {
        bail!("no manifests in {}", models_dir.display());
    }
    manifests.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    if !dry_run {
        std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("cannot create {}", out_dir.display()))?;
    }

    for manifest in &manifests {
        // `models/<id>.md` beside the manifest is the upstream README, if somebody has saved one.
        let readme_path = models_dir.join(format!("{}.md", manifest.id));
        let readme = std::fs::read_to_string(&readme_path).ok();
        let page = summo_models::page::render(manifest, readme.as_deref());

        let target = out_dir.join(summo_models::page::file_name(manifest));
        if dry_run {
            println!("would write {} ({} bytes)", target.display(), page.len());
        } else {
            std::fs::write(&target, page)
                .with_context(|| format!("cannot write {}", target.display()))?;
            println!("wrote {}", target.display());
        }
    }

    let index = pages_index(&manifests);
    let index_path = out_dir.join("README.md");
    if dry_run {
        println!("would write {}", index_path.display());
    } else {
        std::fs::write(&index_path, index)
            .with_context(|| format!("cannot write {}", index_path.display()))?;
        println!("wrote {}", index_path.display());
    }

    println!("\n{} model page(s)", manifests.len());
    Ok(())
}

/// The index page: one row per model, with the two facts that decide whether to read further.
fn pages_index(manifests: &[Manifest]) -> String {
    let mut out = String::from(
        "# Models

",
    );
    out.push_str(
        "Everything here is generated from the manifests in `models/`. Speech recognition and \
         speaker attribution always run on the user's own machine; these are the files that make \
         that possible.

",
    );
    out.push_str(
        "| Model | Task | Size | Licence | Source |
|---|---|---|---|---|
",
    );

    for m in manifests {
        let source = if m.gated {
            "upstream, gated"
        } else if m.redistributable {
            "mirrored"
        } else {
            "upstream"
        };
        out.push_str(&format!(
            "| [{}]({}) | {} | {} | {} | {} |\n",
            m.name,
            summo_models::page::file_name(m),
            summo_models::page::task_name(m.task),
            summo_models::page::human_bytes(m.size_bytes),
            m.license,
            source,
        ));
    }
    out.push('\n');
    out
}

/// Run the daemon and the interface until interrupted.
#[cfg(feature = "serve")]
async fn serve(paths: &Paths, port: u16, no_open: bool, dev: bool) -> Result<()> {
    let engine = summo_engine::EngineState::new(paths.clone())?;
    let server = summo_engine::Server::start(
        engine,
        summo_engine::ServerConfig {
            port,
            // Written so `summo import`, the desktop shell and anything else on this machine can
            // find the running daemon rather than starting a second one.
            write_token_file: true,
            allow_loopback_origins: dev,
        },
    )
    .await?;

    let url = format!("http://127.0.0.1:{}/", server.addr().port());

    if summo_engine::assets::bundled() {
        println!("Summo đang chạy tại {url}");
    } else {
        // Said plainly rather than left as a blank page: a build without the interface is a
        // perfectly good API server, and the user should know which one they have.
        println!("Daemon đang chạy tại {url} (bản build này không kèm giao diện)");
    }
    println!("Ctrl-C để dừng.");

    if !no_open && summo_engine::assets::bundled() {
        open_browser(&url);
    }

    // Ctrl-C rather than running forever: a recording in progress is flushed on its own interval,
    // so stopping here costs seconds of audio at worst.
    tokio::signal::ctrl_c().await.ok();
    println!("\nDừng.");
    server.shutdown();
    Ok(())
}

/// Open the default browser, and say nothing if there is not one.
///
/// A headless server, a container, an SSH session — all of them have no browser and all of them are
/// legitimate places to run this. The address is already printed, so failing quietly loses nothing.
#[cfg(feature = "serve")]
fn open_browser(url: &str) {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", ""])
    } else {
        ("xdg-open", vec![])
    };

    let opened = std::process::Command::new(program)
        .args(args)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok();

    if !opened {
        tracing::debug!("no browser to open; the address is printed above");
    }
}

/// Serve the vault over stdio until the client closes it.
///
/// One JSON object per line each way. Logging goes to stderr and each reply is flushed: a stray
/// line on stdout is a parse error at the other end, and a reply sitting in a buffer looks like a
/// server that hung.
#[cfg(feature = "mcp")]
fn mcp(paths: &Paths) -> Result<()> {
    use std::io::{BufRead, Write};

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    tracing::info!(vault = %paths.vault().display(), "serving the vault over stdio");

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: summo_mcp::Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(e) => {
                // A malformed line has no id to reply against, so there is nobody to tell but the
                // log. Answering with a null id would be a second protocol error on top of the
                // first.
                tracing::warn!(error = %e, "ignoring an unparseable request");
                continue;
            }
        };

        let Some(response) = summo_mcp::handle(paths, &request) else {
            continue;
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
