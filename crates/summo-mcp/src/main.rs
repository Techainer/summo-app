//! `summo-mcp` — the vault, over stdio, for an MCP client.
//!
//! The loop is [`summo_mcp::serve_stdio`], and it is there rather than here because `summo mcp`
//! ships the same transport. Two copies of a protocol loop agree right up until somebody fixes one
//! of them — and of these two, the shipped one is the other.

use summo_core::paths::Paths;

fn main() -> anyhow::Result<()> {
    // stderr, always. Stdout is a JSON-RPC stream, one object per line, and a log line in it is a
    // parse error the client reports as the server being broken.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "summo_mcp=info".into()),
        )
        .init();

    let paths = Paths::discover()?;
    tracing::info!(vault = %paths.vault().display(), "serving the vault over stdio");

    summo_mcp::serve_stdio(&paths, std::io::stdin().lock(), std::io::stdout().lock())?;
    Ok(())
}
