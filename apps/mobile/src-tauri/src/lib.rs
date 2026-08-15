//! Summo on a phone.
//!
//! The same interface as the desktop app — `apps/web` is the frontend for both — over the same
//! engine. What differs is one thing, and it decides everything else: **the engine is linked in
//! rather than spawned**.
//!
//! iOS does not let an App Store build run another executable. There is no `fork` it may use, no
//! second binary the system will launch, and no sidecar. Android allows it and punishes it: a
//! second process is the first thing killed under memory pressure, which on a phone is most of the
//! time. So `summo_engine::embedded` starts the daemon inside this process and the webview talks to
//! it over loopback exactly as it would to a sidecar — same routes, same token, same protocol.
//!
//! The cost is crash isolation. On desktop a panic in a decoder kills the daemon and the app
//! reconnects to the transcript already on disk; here it takes the app with it. That is why the
//! recorder flushes on its own interval — a crash costs seconds of a meeting, not the meeting.
//!
//! ## What is not decided yet
//!
//! **Recording sources.** Microphone capture works on both platforms and covers in-person meetings.
//! Android's `AudioPlaybackCapture` can take audio from other apps, so Zoom or Meet playing on the
//! same phone can be transcribed — though an app may opt out of being captured, and the meeting
//! apps are exactly the ones that might. iOS has no equivalent.
//!
//! **Call recording is not implementable and is not planned.** iOS has never exposed call audio to
//! third parties, and iOS 18.1's own call recording has no third-party API. Google Play has banned
//! third-party call recording apps since May 2022 — only the OEM dialer may. This is policy and
//! platform, not difficulty, and no amount of work changes it.
//!
//! **Model size against phone memory.** `summo_models::recommend` already scores by available RAM,
//! which is the machinery this needs; what it does not yet have is a measured RTF row for phone
//! CPUs, so the ranking on a phone is currently an extrapolation rather than a measurement.

// `Emitter` as well as `Manager`, and it is not tidiness. Tauri v2 split what v1 had on one trait:
// `Manager` carries `path()` and `state()`, `Emitter` carries `emit()`. Without the second import
// the two `handle.emit(...)` calls below do not compile — which is how far this file had ever been
// taken, because nothing built it.
use tauri::{Emitter, Manager};

/// The engine, held for the life of the app so its task is not dropped.
///
/// The failure is kept beside it rather than only logged. An engine that cannot start is the end
/// of the app, and the interface polls this until it gets an answer — so without somewhere to
/// record *why*, a vault that could not be opened and an engine still opening it look identical
/// forever, and the app waits for something that is never coming.
#[derive(Default)]
struct Engine {
    embedded: std::sync::Mutex<Option<summo_engine::embedded::Embedded>>,
    failure: std::sync::Mutex<Option<String>>,
}

/// What the interface is told when it asks where to connect.
///
/// Returned as a command rather than baked into the URL, because on mobile the webview is created
/// before the async runtime has finished starting the engine — and a page that loads with no port
/// is easier to retry than one loaded with the wrong one. The same three states as the desktop
/// shell's `engine_handshake`, because one interface consumes both.
#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum Status {
    Starting,
    Ready { port: u16, token: String },
    Failed { error: String },
}

#[tauri::command]
fn engine_handshake(engine: tauri::State<'_, Engine>) -> Status {
    // `into_inner` on a poisoned lock: both slots are written whole or not at all, so the value is
    // readable, and refusing to answer would strand the interface polling a lock it cannot fix.
    if let Some(embedded) = engine
        .embedded
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
    {
        return Status::Ready {
            port: embedded.handshake().port,
            token: embedded.handshake().token.clone(),
        };
    }
    match &*engine
        .failure
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        Some(error) => Status::Failed {
            error: error.clone(),
        },
        None => Status::Starting,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Engine::default())
        .invoke_handler(tauri::generate_handler![engine_handshake])
        .setup(|app| {
            // The app's own sandboxed directory. `~/.summo` does not exist on either platform, and
            // anything written outside this path is removed by the OS or refused outright.
            let home = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("no app data directory: {e}"))?;

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match summo_engine::embedded::start(home).await {
                    Ok(embedded) => {
                        let state = handle.state::<Engine>();
                        // Stored before the event: a listener that asks the instant it hears
                        // "ready" must not be told "starting".
                        *state
                            .embedded
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(embedded);
                        // The interface polls `engine_handshake` until it answers, so this is the
                        // signal that it can stop.
                        let _ = handle.emit("summo://engine-ready", ());
                    }
                    Err(e) => {
                        // Nothing works without the engine, and a blank screen is the worst way to
                        // say so. The interface shows this verbatim.
                        *handle
                            .state::<Engine>()
                            .failure
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(e.to_string());
                        let _ = handle.emit("summo://engine-failed", e.to_string());
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Summo");
}
