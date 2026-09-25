//! `summo sync` — one run against a folder.
//!
//! A folder rather than an account, because that is a whole feature on its own: point two machines
//! at the same NAS mount, Dropbox directory or USB stick and they stay in step, with no relay and
//! nobody's server involved. The hosted tier will implement the same [`summo_sync::Remote`] trait;
//! it is a convenience, not the only way in.
//!
//! [`summo_engine::sync`] is the pipeline. This is the terminal in front of it, and the app's
//! storage screen is the other door — the same code either way, which is the point: until that
//! module existed, ninety-four tests' worth of working sync had exactly one caller and no way into
//! the product.
//!
//! The passphrase is read from the environment or prompted for, never taken as an argument: an
//! argument lands in shell history and in the process list, where every other user on the machine
//! can read it.

use anyhow::{Context, Result, bail};
use summo_core::paths::Paths;

/// Where the passphrase comes from when it is not typed.
const ENV_PASSPHRASE: &str = "SUMMO_SYNC_PASSPHRASE";

pub fn run(
    paths: &Paths,
    to: &std::path::Path,
    machine: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    // Before the prompt, so nobody types a passphrase into a run that was going to refuse the
    // folder anyway.
    summo_sync::session::check(&to.display().to_string())
        .with_context(|| format!("cannot sync through {}", to.display()))?;

    let settings = summo_core::Settings::load(&paths.settings()).unwrap_or_default();
    let machine = match machine.map(str::trim).filter(|m| !m.is_empty()) {
        Some(named) => named.to_string(),
        // The screen's answer when the command line did not give one, so the two doors agree on
        // what this machine is called — a conflict copy named by one and read by the other is the
        // one place a disagreement shows up, and it shows up as two files nobody can tell apart.
        None => summo_sync::session::machine_name(&settings.sync),
    };

    let report = summo_sync::session::run(
        paths,
        &summo_sync::session::Request {
            folder: to.display().to_string(),
            passphrase: passphrase()?,
            machine,
            dry_run,
        },
    )?;

    if dry_run {
        for step in &report.steps {
            println!(
                "{:<12} {}",
                summo_sync::session::label(&step.action),
                step.path
            );
        }
    }
    for refused in &report.refused {
        eprintln!("refused a path that would escape the vault: {refused}");
    }
    for conflict in &report.conflicts {
        // Not an error and not silent. The user has two whole files and a decision to make.
        eprintln!(
            "conflict: {} — the other version is beside it as {}",
            conflict.path, conflict.copy
        );
    }
    println!("{}", report.summary);
    Ok(())
}

/// The passphrase, from the environment or from the terminal.
///
/// Never an argument. An argument is in the shell history and in `ps` output, where every other
/// user on the machine can read it — and this passphrase is the only thing between somebody with
/// the relay's storage and every meeting in the vault.
fn passphrase() -> Result<String> {
    if let Ok(from_env) = std::env::var(ENV_PASSPHRASE)
        && !from_env.trim().is_empty()
    {
        return Ok(from_env);
    }

    eprint!("Sync passphrase: ");
    use std::io::Write;
    std::io::stderr().flush().ok();

    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("cannot read the passphrase")?;
    let line = line.trim().to_string();

    if line.is_empty() {
        bail!("a sync passphrase is required. Set {ENV_PASSPHRASE} to avoid the prompt.");
    }
    Ok(line)
}
