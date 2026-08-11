//! `summo-mcp` — the vault, over stdio, for an MCP client.
//!
//! One JSON object per line in, one per line out. That framing is what every MCP stdio client
//! speaks, and it is why logging goes to stderr: a stray line on stdout is a parse error at the
//! other end, and the client reports it as the server being broken rather than as a log line.

use std::io::{BufRead, Write};

use summo_core::paths::Paths;
use summo_mcp::{Request, Response, handle};

fn main() -> anyhow::Result<()> {
    // stderr, always. See the note above.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "summo_mcp=info".into()),
        )
        .init();

    let paths = Paths::discover()?;
    tracing::info!(vault = %paths.vault().display(), "serving the vault over stdio");

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(e) => {
                // A malformed line has no id to reply against, so there is nobody to tell but the
                // log. Answering with a null id would be a second protocol error on top of the
                // first.
                tracing::warn!(error = %e, "ignoring an unparseable request");
                continue;
            }
        };

        let Some(response) = handle(&paths, &request) else {
            continue;
        };
        write(&mut stdout, &response)?;
    }
    Ok(())
}

fn write(out: &mut std::io::Stdout, response: &Response) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *out, response)?;
    out.write_all(b"\n")?;
    // Flushed per message: a client waiting on a reply that is sitting in a buffer looks like a
    // server that hung.
    out.flush()?;
    Ok(())
}
