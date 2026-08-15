//! Starting the daemon, and telling the interface where it is.
//!
//! The window is loaded from the bundle over Tauri's own scheme, so it has no address for the
//! daemon and no way to guess one — the port is chosen at bind time and the token is generated per
//! run. Both come from here, over the one channel a webview on a custom scheme has: a command.
//!
//! ## What was wrong
//!
//! Nothing started the daemon. `tauri.conf.json` has carried `externalBin` since the first commit
//! and `scripts/sidecar.sh` stages the binary into the bundle, but no line of this crate ever
//! spawned it, and no line ever handed the interface a port or a token. The app fell back to
//! `{ port: 8710, token: "" }` — the development default — so the packaged desktop app connected to
//! nothing unless a developer happened to be running `summo serve` unauthenticated on that port.
//! Every browser suite passes because the daemon serves the interface itself there and injects the
//! handshake into the document; the shell is the one path nothing exercised.
//!
//! ## Reusing a daemon that is already up
//!
//! `summo serve` and this app keep their state in the same place, and two daemons writing one
//! vault is a corrupted index rather than a slow app. So an `engine.json` whose port still accepts
//! a connection is adopted as-is, and a sidecar is only spawned when there is nothing answering.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};

/// How long the interface waits before it says the daemon never came up.
///
/// Generous on purpose: the first run of a signed build on macOS is held up by Gatekeeper checking
/// the binary, which on a cold disk is seconds, and the failure this produces is one a user cannot
/// do anything about.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(120);
/// A connect that is refused answers instantly; this only bounds the case where nothing answers.
const PROBE: Duration = Duration::from_millis(400);

/// Where the daemon says it is listening, and the token that proves the caller is this app.
#[derive(Clone, Serialize, Deserialize)]
pub struct Handshake {
    pub port: u16,
    pub token: String,
}

/// What the interface is told when it asks.
///
/// Three states rather than a `Result`, because "not yet" and "never" need different behaviour
/// from the caller and a string it has to match on is not an interface. A failure carries the
/// daemon's own words: whatever it wrote before dying is the only description of what went wrong
/// that anybody will ever see.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Status {
    Starting,
    Ready { port: u16, token: String },
    Failed { error: String },
}

#[derive(Default)]
pub struct Engine {
    outcome: Mutex<Option<Result<Handshake, String>>>,
    /// Held so the daemon can be stopped with the app. Tauri kills tracked children on exit, but
    /// only for a clean exit, and a user who quits from the tray should not leave one running.
    child: Mutex<Option<CommandChild>>,
}

#[tauri::command]
pub fn engine_handshake(engine: State<'_, Engine>) -> Status {
    // A poisoned lock means a panic happened while somebody held it; the value inside is a
    // `Option<Result<…>>` that is either written whole or not at all, so it is still readable.
    match &*engine
        .outcome
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
    {
        None => Status::Starting,
        Some(Ok(handshake)) => Status::Ready {
            port: handshake.port,
            token: handshake.token.clone(),
        },
        Some(Err(error)) => Status::Failed {
            error: error.clone(),
        },
    }
}

/// Bring the daemon up in the background, and announce the result.
///
/// Spawned rather than awaited in `setup`, because the window has to appear first. An app that
/// shows nothing for the two seconds a model directory takes to scan looks broken in exactly the
/// way this product cannot afford on first run.
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = launch(&handle).await;
        // Stored before the event is emitted: a listener that calls `engine_handshake` the instant
        // it hears "ready" must not be told "starting".
        *handle
            .state::<Engine>()
            .outcome
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(outcome.clone());
        match outcome {
            Ok(_) => {
                let _ = handle.emit("summo://engine-ready", ());
            }
            Err(error) => {
                // To the terminal as well as to the window. Somebody running this from a shell to
                // find out why it will not work should not have to read it off a splash screen,
                // and a build where the webview itself is broken has nowhere else to say it.
                eprintln!("summo: {error}");
                let _ = handle.emit("summo://engine-failed", error);
            }
        }
    });
}

/// Stop the daemon this app started. A daemon it adopted is left alone — it belongs to whoever
/// started it, and a terminal running `summo serve` should survive the app quitting.
pub fn stop(app: &AppHandle) {
    let child = app
        .state::<Engine>()
        .child
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some(child) = child {
        let _ = child.kill();
    }
}

async fn launch(app: &AppHandle) -> Result<Handshake, String> {
    let file = home(app)?.join("engine.json");

    if let Some(running) = read(&file)
        && answers(&running).await
    {
        return Ok(running);
    }
    // A file left behind by a daemon that is gone would otherwise be read back below as the new
    // one's address, and every request would go to a port nothing is on.
    std::fs::remove_file(&file).ok();

    let mut command = app
        .shell()
        .sidecar("summo-engine")
        .map_err(|e| format!("this build has no daemon in it: {e}"))?;
    if let Some(path) = library_path(app) {
        command = command.env(LIBRARY_PATH_VAR, path);
    }
    if tauri::is_dev() {
        // In a development run the window is loaded from Vite on another loopback port, and the
        // daemon refuses a request carrying an `Origin` it does not recognise — which is the whole
        // reason that check exists: a page on the open internet must never reach the microphone.
        // `--dev` widens it to loopback pages.
        //
        // Only here. A packaged app's page comes from `tauri://localhost`, which the daemon already
        // trusts by scheme, so the flag would buy nothing and would trust every port on the
        // machine, including whatever the user has running on 3000.
        command = command.arg("--dev");
    }
    let (mut events, child) = command
        .spawn()
        .map_err(|e| format!("cannot start the daemon: {e}"))?;
    *app.state::<Engine>()
        .child
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(child);

    let deadline = Instant::now() + READY_TIMEOUT;
    // Everything the daemon printed, kept so a failure can be reported in its words rather than as
    // a timeout. A daemon that exits because a port is taken or a vault is unreadable says so.
    let mut said = String::new();
    let outcome = 'ready: loop {
        while let Ok(event) = events.try_recv() {
            match event {
                CommandEvent::Stdout(line) | CommandEvent::Stderr(line) => {
                    said.push_str(String::from_utf8_lossy(&line).trim_end());
                    said.push('\n');
                }
                CommandEvent::Terminated(status) => {
                    break 'ready Err(format!(
                        "the daemon stopped before it was ready (exit {}).\n{}",
                        status.code.unwrap_or(-1),
                        said.trim()
                    ));
                }
                CommandEvent::Error(error) => break 'ready Err(error),
                _ => {}
            }
        }

        if let Some(handshake) = read(&file)
            && answers(&handshake).await
        {
            break Ok(handshake);
        }

        if Instant::now() >= deadline {
            break Err(format!(
                "the daemon did not come up in {}s.\n{}",
                READY_TIMEOUT.as_secs(),
                said.trim()
            ));
        }
        tokio::time::sleep(POLL).await;
    };

    // Keep reading for the life of the process, and throw it away.
    //
    // Not tidiness: the daemon logs a line per request, and a pipe nobody reads fills at 64 kB and
    // blocks the process writing into it. Dropping the receiver here would freeze the engine after
    // a few hundred requests — an app that works for a minute and then stops, which is a far worse
    // bug than the one this whole file exists to fix.
    tauri::async_runtime::spawn(async move { while events.recv().await.is_some() {} });

    outcome
}

/// What the loader reads to find a library that is not in a system directory.
#[cfg(target_os = "macos")]
const LIBRARY_PATH_VAR: &str = "DYLD_LIBRARY_PATH";
#[cfg(target_os = "windows")]
const LIBRARY_PATH_VAR: &str = "PATH";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const LIBRARY_PATH_VAR: &str = "LD_LIBRARY_PATH";

/// Where the daemon's own libraries are, prepended to whatever the user's environment says.
///
/// The daemon is not self-contained and cannot be: ONNX Runtime and sherpa-onnx are C++ libraries
/// loaded at start, and it is linked against them with a runpath of `$ORIGIN` — beside me. That
/// works in the CLI tarball, where `scripts/bundle.sh` puts them beside it, and it did not work
/// here, where `scripts/sidecar.sh` staged the executable alone. The result was a bundle that
/// installed, opened, and died at
///
///   error while loading shared libraries: libsherpa-onnx-c-api.so
///
/// the moment it tried to start its engine. They are shipped as bundle resources now, and the
/// resource directory is not the directory the sidecar is in on Linux or macOS — hence this,
/// rather than relying on `$ORIGIN` a second time.
///
/// The sidecar's own directory is included too, because on Windows it is the install directory and
/// on a development run it is where `tauri-build` puts the staged copy.
///
/// Untested on a signed macOS build: the hardened runtime strips `DYLD_*` from a child process, so
/// a notarised bundle will need the dylibs beside the sidecar in `Contents/MacOS` instead. Nothing
/// in this repository can sign one, so that is a note rather than a fix.
fn library_path(app: &AppHandle) -> Option<String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(resources) = app.path().resource_dir() {
        // `lib/` is where `tauri.conf.json` puts them; the resource root itself is listed too, for
        // a development run where nothing has copied anything anywhere.
        dirs.push(resources.join("lib"));
        dirs.push(resources);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(beside) = exe.parent()
    {
        dirs.push(beside.to_path_buf());
    }
    if let Some(existing) = std::env::var_os(LIBRARY_PATH_VAR) {
        dirs.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(dirs)
        .ok()
        .map(|joined| joined.to_string_lossy().into_owned())
}

/// Where the daemon keeps its state, by the same rule the daemon itself uses.
///
/// `SUMMO_HOME` first: the sidecar inherits this process's environment, so a portable install or a
/// test harness that sets it would otherwise have the app looking in `~/.summo` for a file the
/// daemon wrote somewhere else.
pub fn home(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("SUMMO_HOME") {
        return Ok(PathBuf::from(dir));
    }
    app.path()
        .home_dir()
        .map(|home| home.join(".summo"))
        .map_err(|e| format!("no home directory to keep the vault in: {e}"))
}

fn read(file: &std::path::Path) -> Option<Handshake> {
    let handshake: Handshake = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    // Half a handshake is not a connection: a token-less daemon would answer 401 to everything.
    (handshake.port != 0 && !handshake.token.is_empty()).then_some(handshake)
}

/// Whether anything is listening on that port.
///
/// A connection, not a request: the daemon refuses an unauthenticated caller, so a 401 and a
/// working daemon look the same from here — and this only has to answer "is it gone".
async fn answers(handshake: &Handshake) -> bool {
    tokio::time::timeout(
        PROBE,
        tokio::net::TcpStream::connect(("127.0.0.1", handshake.port)),
    )
    .await
    .is_ok_and(|attempt| attempt.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_a_handshake_is_not_one() {
        let dir = std::env::temp_dir().join(format!("summo-shell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("engine.json");

        std::fs::write(&file, br#"{"port":0,"token":"abc"}"#).unwrap();
        assert!(read(&file).is_none(), "a port of zero is not an address");

        std::fs::write(&file, br#"{"port":8710,"token":""}"#).unwrap();
        assert!(read(&file).is_none(), "a daemon with no token refuses us");

        std::fs::write(&file, br#"{"port":8710,"token":"abc","pid":7}"#).unwrap();
        let found = read(&file).expect("the daemon writes more fields than this needs");
        assert_eq!(found.port, 8710);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_port_with_nothing_on_it_does_not_answer() {
        // Bound and dropped, so the port is real and free — the shape of a stale `engine.json`.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        assert!(
            !answers(&Handshake {
                port,
                token: "abc".into()
            })
            .await
        );
    }
}
