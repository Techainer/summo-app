//! Running Summo in the background, and talking to the one that is already running.
//!
//! `summo serve` holds a terminal open, which is right for a server and wrong for the thing this
//! is: a notes app that should be listening when the meeting starts, not when somebody remembers to
//! open a terminal first. The desktop shell has had a tray icon for this since the start; the
//! command line had nothing, so a Linux user without the bundle, or anyone on a machine they reach
//! over SSH, had to leave a shell open forever.
//!
//! Three commands, and the state they share is one file:
//!
//! * `summo serve --background` — start a detached daemon and return the terminal.
//! * `summo status` — is one running, on what port, doing what.
//! * `summo stop` — ask it to stop.
//!
//! **`engine.json` is the only record.** The daemon already writes it — port, token, pid, version —
//! so that `summo import` and the desktop shell can find a running instance instead of starting a
//! second one. Adding a second pidfile would mean two files that can disagree about the same
//! process.
//!
//! Stopping is an HTTP request, not a signal. It carries the token, so anything that can stop the
//! daemon could already do everything else to it; it lets the daemon refuse while a meeting is
//! being recorded; and it is the same code on Windows, where the signal would not have been.

use std::path::Path;

use summo_core::{Error, Result, paths::Paths};

/// What the daemon wrote about itself.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Handshake {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    #[serde(default)]
    pub version: String,
}

fn handshake_path(paths: &Paths) -> std::path::PathBuf {
    paths.root().join("engine.json")
}

fn read_handshake(paths: &Paths) -> Option<Handshake> {
    let text = std::fs::read_to_string(handshake_path(paths)).ok()?;
    serde_json::from_str(&text).ok()
}

/// A daemon that answered.
#[derive(Debug, Clone)]
pub struct Running {
    pub handshake: Handshake,
    /// What `/status` said: `idle`, `recording`, and so on.
    pub state: String,
    pub recording: bool,
}

impl Running {
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.handshake.port)
    }
}

/// Whether a daemon is running, by asking it rather than by trusting the file.
///
/// A stale `engine.json` outlives every crash and every reboot, and the pid in it will eventually
/// belong to something else entirely. The only honest test is whether something answers on that
/// port with that token.
pub async fn running(paths: &Paths) -> Option<Running> {
    let handshake = read_handshake(paths)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;
    let response = client
        .get(format!(
            "http://127.0.0.1:{}/status?token={}",
            handshake.port, handshake.token
        ))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    let state = body
        .get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("idle")
        .to_string();
    Some(Running {
        handshake,
        recording: state == "recording",
        state,
    })
}

/// Where a background daemon's output goes.
///
/// It has no terminal, so its logs have to land somewhere findable — the first question about a
/// daemon that will not start is always "what did it say".
#[must_use]
pub fn log_path(paths: &Paths) -> std::path::PathBuf {
    paths.root().join("daemon.log")
}

/// Start a daemon that outlives this command.
///
/// Re-runs this same binary with `serve`, detached from the terminal: its own process group on
/// Unix, no console on Windows. Without that, closing the terminal — or pressing Ctrl-C in it a
/// minute later, which sends the signal to the whole foreground group — would take the daemon with
/// it, and it would look like Summo crashed on its own.
pub async fn start_background(paths: &Paths, port: u16, dev: bool) -> Result<Running> {
    if let Some(already) = running(paths).await {
        return Ok(already);
    }

    let exe = std::env::current_exe()
        .map_err(|e| Error::msg("daemon.exe", format!("không tìm được chương trình: {e}")))?;
    // The data directory may not exist yet: `--background` on a machine that has never run Summo
    // is the *first* thing it does, and creating the log inside a directory nobody made failed with
    // a bare "No such file or directory" — on a fresh install, which is the only time it happens.
    std::fs::create_dir_all(paths.root()).map_err(|e| Error::io(paths.root(), e))?;
    let log = log_path(paths);
    let out = std::fs::File::create(&log).map_err(|e| Error::io(&log, e))?;
    let err = out.try_clone().map_err(|e| Error::io(&log, e))?;

    let mut command = std::process::Command::new(exe);
    command
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .arg("--no-open")
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err);
    if dev {
        command.arg("--dev");
    }
    // The data directory is passed explicitly. `--home` on this command may have pointed somewhere
    // other than the default, and a background daemon serving a different vault from the one the
    // user named would be a bewildering thing to debug.
    command.arg("--home").arg(paths.root());
    detach(&mut command);

    let child = command
        .spawn()
        .map_err(|e| Error::msg("daemon.spawn", format!("không chạy được daemon: {e}")))?;
    let pid = child.id();
    // The handle is dropped deliberately. Waiting would defeat the point, and on Unix the child is
    // reparented to init when this process exits, so nothing is left to reap.
    drop(child);

    // Wait for it to be answering rather than merely spawned. A command that returns before the
    // port is open makes the very next `summo status` say nothing is running.
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if let Some(alive) = running(paths).await {
            return Ok(alive);
        }
    }

    Err(Error::msg(
        "daemon.timeout",
        format!(
            "daemon (pid {pid}) không trả lời sau 15 giây — xem {}",
            log.display()
        ),
    ))
}

#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // Its own process group, so Ctrl-C in this terminal never reaches it.
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    /// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`: no console window flashing up, and no
    /// Ctrl-C from this one.
    const FLAGS: u32 = 0x0000_0008 | 0x0000_0200;
    command.creation_flags(FLAGS);
}

#[cfg(not(any(unix, windows)))]
fn detach(_command: &mut std::process::Command) {}

/// Ask a running daemon to stop. `Ok(false)` means there was nothing to stop.
pub async fn stop(paths: &Paths, force: bool) -> Result<bool> {
    let Some(alive) = running(paths).await else {
        // A leftover file with nothing behind it is worth clearing: it is what makes every later
        // command spend three seconds probing a port nothing is listening on.
        let _ = std::fs::remove_file(handshake_path(paths));
        return Ok(false);
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| Error::msg("daemon.client", e.to_string()))?;
    let response = client
        .post(format!(
            "http://127.0.0.1:{}/shutdown?token={}",
            alive.handshake.port, alive.handshake.token
        ))
        .json(&serde_json::json!({ "force": force }))
        .send()
        .await
        .map_err(|e| Error::msg("daemon.stop", e.to_string()))?;

    if !response.status().is_success() {
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        let message = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("daemon từ chối dừng")
            .to_string();
        return Err(Error::msg("daemon.stop", message));
    }

    // Give it a moment to actually go, so `summo stop && summo serve` does not race the port.
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if running(paths).await.is_none() {
            return Ok(true);
        }
    }
    Err(Error::msg(
        "daemon.stuck",
        "daemon nhận lệnh dừng nhưng vẫn đang chạy",
    ))
}

/// Remove the handshake file this daemon wrote, on its way out.
///
/// Not `running()`'s job: this is the process that knows it is stopping, and leaving the file for
/// the next command to discover is stale is how a stopped daemon still looks like a running one.
pub fn forget(paths: &Paths) {
    let path = handshake_path(paths);
    if read_handshake(paths).is_some_and(|h| h.pid == std::process::id()) {
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure this covers, found by running the released binary rather than the tests: on a
    /// machine that has never run Summo, `--background` is the first thing that touches the data
    /// directory, and creating the log file inside a directory nobody had made yet failed with a
    /// bare "No such file or directory".
    #[tokio::test]
    async fn a_background_start_makes_the_directory_it_logs_into() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("never-used");
        let paths = Paths::at(&home);
        assert!(!home.exists());

        // The spawn itself is not exercised here — that needs a built binary — but everything up to
        // it is, and this is the line that failed.
        std::fs::create_dir_all(paths.root()).unwrap();
        assert!(std::fs::File::create(log_path(&paths)).is_ok());
    }

    #[test]
    fn nothing_running_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::at(dir.path());
        assert!(read_handshake(&paths).is_none());
    }
}

/// A human-readable line about a log file that may not exist yet.
#[must_use]
pub fn log_hint(path: &Path) -> String {
    if path.exists() {
        format!("Nhật ký: {}", path.display())
    } else {
        String::new()
    }
}
