//! `summo-engine` — the local daemon.

use anyhow::{Context, Result};
use clap::Parser;
use summo_core::paths::Paths;
use summo_engine::{EngineState, Server, ServerConfig};

#[derive(Parser)]
#[command(name = "summo-engine", version, about)]
struct Cli {
    /// Data directory. Defaults to the platform application-data path, or `SUMMO_HOME`.
    #[arg(long)]
    home: Option<std::path::PathBuf>,

    /// Port to bind on loopback. 0 picks an ephemeral one, which is the default because a
    /// well-known port is something other programs can find and probe.
    #[arg(long, default_value_t = 0)]
    port: u16,

    /// Print the address and token and exit, instead of serving. For scripting.
    #[arg(long)]
    print_handshake: bool,
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

    let engine = EngineState::new(paths.clone()).context("cannot initialise engine state")?;
    let server = Server::start(
        engine,
        ServerConfig {
            port: cli.port,
            write_token_file: true,
        },
    )
    .await
    .context("cannot start the engine")?;

    println!("summo-engine listening on http://{}", server.addr());
    println!(
        "handshake written to {}",
        paths.root().join("engine.json").display()
    );

    if cli.print_handshake {
        return Ok(());
    }

    // Shut down cleanly on Ctrl-C so the token file does not outlive the process it belongs to.
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutting down");
    std::fs::remove_file(paths.root().join("engine.json")).ok();
    std::fs::remove_file(summo_engine::auth::token_path(paths.root())).ok();
    server.shutdown();
    Ok(())
}
