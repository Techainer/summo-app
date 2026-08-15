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
struct Engine(std::sync::Mutex<Option<summo_engine::embedded::Embedded>>);

/// Where the interface should connect.
///
/// Returned to the webview as a command rather than baked into the URL, because on mobile the
/// webview is created before the async runtime has finished starting the engine — and a page that
/// loads with no port is easier to retry than one loaded with the wrong one.
#[tauri::command]
fn handshake(engine: tauri::State<'_, Engine>) -> Result<serde_json::Value, String> {
    let guard = engine.0.lock().map_err(|_| "engine lock poisoned".to_string())?;
    let embedded = guard
        .as_ref()
        .ok_or_else(|| "the engine is still starting".to_string())?;
    Ok(serde_json::json!({
        "port": embedded.handshake().port,
        "token": embedded.handshake().token,
    }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Engine(std::sync::Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![handshake])
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
                        if let Ok(mut slot) = state.0.lock() {
                            *slot = Some(embedded);
                        }
                        // The interface polls `handshake` until it answers, so this is the signal
                        // that it can stop.
                        let _ = handle.emit("summo://engine-ready", ());
                    }
                    Err(e) => {
                        // Nothing works without the engine, and a blank screen is the worst way to
                        // say so. The interface shows this verbatim.
                        let _ = handle.emit("summo://engine-failed", e.to_string());
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Summo");
}
