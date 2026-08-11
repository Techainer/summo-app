//! `summo sync` — one run against a folder.
//!
//! A folder rather than an account, because that is a whole feature on its own: point two machines
//! at the same NAS mount, Dropbox directory or USB stick and they stay in step, with no relay and
//! nobody's server involved. The hosted tier will implement the same [`summo_sync::Remote`] trait;
//! it is a convenience, not the only way in.
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
    let state = paths.root().join("sync");
    std::fs::create_dir_all(&state)
        .with_context(|| format!("cannot create {}", state.display()))?;

    let machine = machine
        .map(str::to_string)
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "this-machine".to_string());

    let mut remote = summo_sync::remote::Directory::open(to)
        .with_context(|| format!("cannot open {}", to.display()))?;

    let salt = salt(&mut remote)?;
    let passphrase = passphrase()?;
    let key = summo_sync::Key::derive(&passphrase, &salt)?;

    if dry_run {
        // The plan, without touching anything. Worth having: the first sync of an existing vault
        // moves everything, and somebody should be able to look before that happens.
        let local = summo_sync::Snapshot::scan(&paths.vault())?;
        let base = summo_sync::Snapshot::read(&state.join("base.json"));
        let theirs = remote_snapshot(&remote, &key)?;

        let plan = summo_sync::plan(&local, &theirs, &base);
        for step in &plan.steps {
            println!("{:<12} {}", label(&step.action), step.path);
        }
        for refused in &plan.refused {
            eprintln!("refused      {refused}");
        }
        println!("\n{}", plan.summary());
        return Ok(());
    }

    let outcome = summo_sync::sync(&paths.vault(), &state, &mut remote, &key, &machine)?;

    println!("{}", outcome.summary);
    for conflict in &outcome.conflicts {
        // Not an error and not silent. The user has two whole files and a decision to make.
        eprintln!(
            "conflict: {} — the other version is beside it as {}",
            conflict.path, conflict.copy
        );
    }
    for refused in &outcome.refused {
        eprintln!("refused a path that would escape the vault: {refused}");
    }
    Ok(())
}

fn remote_snapshot(
    remote: &dyn summo_sync::Remote,
    key: &summo_sync::Key,
) -> Result<summo_sync::Snapshot> {
    let Some(bytes) = remote.manifest()? else {
        return Ok(summo_sync::Snapshot::default());
    };
    let sealed = summo_sync::Sealed::from_bytes(&bytes)?;
    let plain = key.open("\u{0}summo-manifest", &sealed)?;
    let manifest: summo_sync::Manifest =
        serde_json::from_slice(&plain).context("the remote manifest is malformed")?;
    Ok(manifest.to_snapshot())
}

fn label(action: &summo_sync::Action) -> &'static str {
    use summo_sync::{Action, Side};
    match action {
        Action::Upload => "upload",
        Action::Download => "download",
        Action::Merge => "merge",
        Action::DeleteRemote => "delete there",
        Action::DeleteLocal => "delete here",
        Action::Resurrect {
            edited_on: Side::Local,
        } => "restore there",
        Action::Resurrect {
            edited_on: Side::Remote,
        } => "restore here",
    }
}

/// The vault's salt, from the remote, created there on the first sync.
///
/// On the remote because it belongs to the shared vault rather than to a machine: a salt per
/// machine means a different key per machine from the same passphrase, and the second machine to
/// sync cannot read anything the first wrote.
///
/// Not a secret, and stored in the clear. It exists so two people who chose the same passphrase do
/// not derive the same key, and so one precomputed table cannot cover both.
fn salt(remote: &mut dyn summo_sync::Remote) -> Result<Vec<u8>> {
    if let Some(existing) = remote.salt()?
        && existing.len() >= 8
    {
        return Ok(existing);
    }
    let fresh = summo_sync::crypto::new_salt()?;
    remote
        .put_salt(&fresh)
        .context("cannot write the sync salt")?;
    Ok(fresh.to_vec())
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
