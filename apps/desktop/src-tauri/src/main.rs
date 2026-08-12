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

/// The three words the tray menu needs, in the language the user chose.
///
/// These were hardcoded Vietnamese, which made the shell the one place in the product that ignored
/// the language setting — the interface it wraps had just been taught not to do that.
///
/// A three-entry table rather than a catalogue. The shell is deliberately thin: it owns a window, a
/// tray and a shortcut, and it is not worth a dependency on the workspace — which is excluded on
/// purpose, since building it needs platform webview libraries — to translate three strings. A
/// language the table has never heard of falls back to English rather than to nothing.
fn tray_words(language: &str) -> [&'static str; 3] {
    // Matched on the primary subtag, so `zh-CN` and `zh-Hans` land on Chinese rather than silently
    // on English. The tray is often the only Summo a user sees for hours; it should not be the one
    // surface still speaking a language they did not pick.
    match language.split(['-', '_']).next().unwrap_or(language) {
        "vi" => ["Ghi ngay", "Mở Summo", "Thoát"],
        "ja" => ["すぐ録音", "Summo を開く", "終了"],
        "zh" => ["立即录音", "打开 Summo", "退出"],
        _ => ["Record now", "Open Summo", "Quit"],
    }
}

/// `interface.language` from the settings file the daemon and the app already share.
///
/// Read directly rather than through `summo-core`, for the same reason as above. Every failure —
/// no file, unreadable, not JSON, no such field — is the same answer: the default. The app has to
/// start on a machine that has never run it.
fn chosen_language(app: &tauri::AppHandle) -> String {
    let settings = app
        .path()
        .home_dir()
        .map(|home| home.join(".summo").join("settings.json"));
    let Ok(path) = settings else {
        return "vi".into();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| {
            json.get("interface")?
                .get("language")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "vi".into())
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let [record_label, open_label, quit_label] = tray_words(&chosen_language(app));
    let record = MenuItem::with_id(app, "record", record_label, true, Some("CmdOrCtrl+Shift+R"))?;
    let open = MenuItem::with_id(app, "open", open_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&record, &open, &quit])?;

    TrayIconBuilder::with_id("summo")
        .menu(&menu)
        // Without this the tray entry is a blank space. The app hides here rather than quitting, so
        // a tray with nothing in it means a running app the user cannot find and cannot stop.
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("no window icon to use in the tray".into())
        })?)
        .icon_as_template(true)
        .tooltip("Summo")
        // A left click shows the window. Every tray app behaves this way, and requiring the menu
        // for the one thing people want from a tray icon is a small daily annoyance.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
                && let Some(window) = tray.app_handle().get_webview_window("main")
            {
                let _ = window.show();
                let _ = window.set_focus();
            }
        })
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
