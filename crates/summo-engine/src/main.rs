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

    /// Accept requests from pages served on this machine.
    ///
    /// For developing the interface against a Vite server, and for browser tests. A shipped build
    /// must never run this way: it is the check that stops a web page reaching your microphone.
    #[arg(long)]
    dev: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    // Before anything loads a model: on Intel macOS the runtime is a file beside this binary
    // rather than one linked at build time, and `ort` has to be told so before its first use.
    if let Some(runtime) = summo_core::onnx::locate_runtime() {
        tracing::debug!(path = %runtime.display(), "using the ONNX Runtime shipped with the app");
    }

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
            allow_loopback_origins: cli.dev,
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

    // Ctrl-C, or `summo stop` reaching this daemon over HTTP.
    //
    // This waited on Ctrl-C alone, and `Server::stop_requested`'s own documentation says it is
    // "awaited beside Ctrl-C, so a daemon started in the background and one started in a terminal
    // stop the same way" — true of `summo serve`, and not of this binary. So `/shutdown` answered
    // `{"stopping": true}`, nothing was listening, and the process ran on: `summo stop` waited five
    // seconds and reported that the daemon had taken the order and stayed. Starting it again then
    // overwrote `engine.json`, leaving the first one serving a port no command could find.
    //
    // This is the binary the desktop app spawns as its sidecar, so the shutdown route existing and
    // doing nothing was the state on every desktop install.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("interrupted"),
        () = server.stop_requested() => tracing::info!("stop requested over HTTP"),
    }
    tracing::info!("shutting down");
    std::fs::remove_file(paths.root().join("engine.json")).ok();
    std::fs::remove_file(summo_engine::auth::token_path(paths.root())).ok();
    server.shutdown();
    Ok(())
}
