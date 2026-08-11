//! The desktop shell.
//!
//! Thin on purpose. Everything that touches audio or a model lives in `summo-engine`, which this
//! process spawns as a sidecar and talks to over a loopback socket. The shell's whole job is to
//! own the window, the tray icon and the global shortcut — the three things that must work before
//! the user has decided to look at the app.
//!
//! The global shortcut is the reason the tray exists at all: the promise is that pressing record
//! starts a recording in under a second, and that cannot be true if the app has to be focused first.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

/// Toggle recording from anywhere. Chosen to avoid the system shortcuts on all three platforms.
///
/// A function rather than a `const`: `Shortcut::new` is not `const fn`, and the compiler is right
/// that it cannot be called in a constant. Building it twice costs nothing — it is two enum values
/// and a bitflag.
fn record_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyR)
}

fn main() {
    tauri::Builder::default()
        // The file dialog for `Nhập file`. The webview only ever gets a *path* back, never the
        // bytes: the daemon reads the file itself, so a two-hour video never crosses the IPC
        // boundary.
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    // Only act on press; firing on release too would toggle twice per keystroke.
                    if shortcut == &record_shortcut() && event.state() == ShortcutState::Pressed {
                        let _ = app.emit("summo://toggle-record", ());
                    }
                })
                .build(),
        )
        .setup(|app| {
            app.global_shortcut().register(record_shortcut())?;
            build_tray(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window ends the *window*, not the session. A recording in progress must
            // survive someone tidying their desktop, so the app hides instead of quitting.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start Summo");
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let record = MenuItem::with_id(app, "record", "Ghi ngay", true, Some("CmdOrCtrl+Shift+R"))?;
    let open = MenuItem::with_id(app, "open", "Mở Summo", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Thoát", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&record, &open, &quit])?;

    TrayIconBuilder::with_id("summo")
        .menu(&menu)
        .tooltip("Summo")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "record" => {
                let _ = app.emit("summo://toggle-record", ());
            }
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
