//! The desktop shell.
//!
//! Thin on purpose. Everything that touches audio or a model lives in `summo-engine`, which this
//! process spawns as a sidecar and talks to over a loopback socket — see `engine.rs`, which is
//! where that spawning finally happens. The rest of the shell's job is to own the window, the tray
//! icon and the global shortcut — the three things that must work before the user has decided to
//! look at the app.
//!
//! The global shortcut is the reason the tray exists at all: the promise is that pressing record
//! starts a recording in under a second, and that cannot be true if the app has to be focused first.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod window;

use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Toggle recording from anywhere. Chosen to avoid the system shortcuts on all three platforms.
///
/// A function rather than a `const`: `Shortcut::new` is not `const fn`, and the compiler is right
/// that it cannot be called in a constant. Building it twice costs nothing — it is two enum values
/// and a bitflag.
fn record_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyR)
}

fn main() {
    let app = tauri::Builder::default()
        // The file dialog for `Nhập file`. The webview only ever gets a *path* back, never the
        // bytes: the daemon reads the file itself, so a two-hour video never crosses the IPC
        // boundary.
        .plugin(tauri_plugin_dialog::init())
        // Spawning the bundled daemon. Nothing in the webview may run a command — see the
        // capability file, which grants the shell plugin no permission at all — so this is the
        // Rust side of the plugin only.
        .plugin(tauri_plugin_shell::init())
        .manage(engine::Engine::default())
        .manage(window::Restore::default())
        .invoke_handler(tauri::generate_handler![
            engine::engine_handshake,
            window::set_shape,
            window::can_float
        ])
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
            // A shortcut that is already taken is not a reason to refuse to start.
            //
            // On Windows a global hotkey is exclusive: whoever registers ⊞+Shift+R first owns it,
            // and everybody after them gets `HotKey already registered`. That was propagated out of
            // the setup hook, which Tauri turns into a panic — so the app died on launch, before a
            // window, with a message only a terminal would show. It happens to anybody who has that
            // combination bound to something else, and to anybody who launches Summo twice.
            //
            // Found the first time this app was ever started on Windows, by the release job.
            if let Err(e) = app.global_shortcut().register(record_shortcut()) {
                eprintln!(
                    "summo: the global record shortcut is not available ({e}). \
                     Everything else works; use the record button in the window."
                );
            }
            build_tray(app.handle())?;
            // A menu bar that fails to build is not a reason to refuse to start: the same rule the
            // global shortcut is under, for the same reason — everything the app does is reachable
            // from inside the window.
            if let Err(e) = build_menu(app.handle()) {
                eprintln!("summo: the menu bar could not be built ({e}). Everything else works.");
            }
            engine::start(app.handle());
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
        .build(tauri::generate_context!())
        .expect("failed to start Summo");

    app.run(|app, event| {
        // The window hides on close, so this is reached only by `Thoát` in the tray or by the OS
        // ending the session. Either way the daemon this app started should go with it: a
        // microphone held open by a process with no window is the worst thing this app could leave
        // behind.
        if let tauri::RunEvent::Exit = event {
            engine::stop(app);
        }
    });
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
    // Through `engine::home`, so the tray reads the settings of the vault the daemon is actually
    // using. These were two copies of the same rule and one of them had never heard of
    // `SUMMO_HOME`: a portable install would have had the app on one vault and its own tray menu
    // reading the language out of another.
    let Ok(root) = engine::home(app) else {
        return "vi".into();
    };
    std::fs::read_to_string(root.join("settings.json"))
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
        // Its own icon, not the application icon.
        //
        // This used to be `default_window_icon()` — the dark rounded square with three green bars,
        // which is right for a dock and wrong for a menu bar. `icon_as_template` tells macOS to
        // ignore the colours and draw the **alpha channel** in the bar's own colour, so it adapts
        // to light and dark; hand it a picture whose alpha is a filled square and it faithfully
        // draws a filled square. A user reported exactly that: a grey square where the logo goes.
        //
        // `icons/tray.png` is the three bars and nothing behind them, drawn by
        // `scripts/tray-icon.py`. Embedded rather than read from the resource directory, because a
        // tray icon that depends on files being where they were installed is a tray icon that
        // disappears on the one machine nobody tested.
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/tray.png"
        ))?)
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

/// The words the menu bar needs, in the language the user chose.
///
/// The same three-row table the tray uses and for the same reason: the shell is deliberately thin
/// and does not depend on the workspace, so this is a list rather than a catalogue. A language it
/// has never heard of falls back to English rather than to nothing.
///
/// Ordered as the menus are built, so adding an item means adding a column here and the compiler
/// says where.
struct MenuWords {
    file: &'static str,
    edit: &'static str,
    view: &'static str,
    help: &'static str,
    new_note: &'static str,
    import: &'static str,
    record: &'static str,
    home: &'static str,
    library: &'static str,
    tasks: &'static str,
    analytics: &'static str,
    settings: &'static str,
    sidebar: &'static str,
    search: &'static str,
    shortcuts: &'static str,
    vault: &'static str,
    docs: &'static str,
    issue: &'static str,
}

fn menu_words(language: &str) -> MenuWords {
    match language.split(['-', '_']).next().unwrap_or(language) {
        "vi" => MenuWords {
            file: "Tệp",
            edit: "Sửa",
            view: "Xem",
            help: "Trợ giúp",
            new_note: "Ghi chú mới",
            import: "Nhập file…",
            record: "Ghi / Dừng",
            home: "Trang chính",
            library: "Kho",
            tasks: "Việc cần làm",
            analytics: "Thống kê",
            settings: "Cài đặt",
            sidebar: "Ẩn/hiện thanh bên",
            search: "Tìm mọi thứ",
            shortcuts: "Phím tắt",
            vault: "Kho trên đĩa",
            docs: "Tài liệu",
            issue: "Báo lỗi",
        },
        "ja" => MenuWords {
            file: "ファイル",
            edit: "編集",
            view: "表示",
            help: "ヘルプ",
            new_note: "新規ノート",
            import: "ファイルを取り込む…",
            record: "録音 / 停止",
            home: "ホーム",
            library: "ライブラリ",
            tasks: "タスク",
            analytics: "統計",
            settings: "設定",
            sidebar: "サイドバーの表示切替",
            search: "すべて検索",
            shortcuts: "キーボードショートカット",
            vault: "保管フォルダ",
            docs: "ドキュメント",
            issue: "問題を報告",
        },
        "zh" => MenuWords {
            file: "文件",
            edit: "编辑",
            view: "视图",
            help: "帮助",
            new_note: "新建笔记",
            import: "导入文件…",
            record: "录制 / 停止",
            home: "主页",
            library: "资料库",
            tasks: "任务",
            analytics: "统计",
            settings: "设置",
            sidebar: "显示/隐藏侧边栏",
            search: "搜索全部",
            shortcuts: "键盘快捷键",
            vault: "磁盘上的保管库",
            docs: "文档",
            issue: "报告问题",
        },
        _ => MenuWords {
            file: "File",
            edit: "Edit",
            view: "View",
            help: "Help",
            new_note: "New note",
            import: "Import a file…",
            record: "Record / Stop",
            home: "Home",
            library: "Library",
            tasks: "Tasks",
            analytics: "Analytics",
            settings: "Settings",
            sidebar: "Toggle sidebar",
            search: "Search everything",
            shortcuts: "Keyboard shortcuts",
            vault: "Vault on disk",
            docs: "Documentation",
            issue: "Report a problem",
        },
    }
}

/// The menu bar.
///
/// The window had none. On macOS that is not a stylistic choice — an app with no menu bar has no
/// **Edit** menu, and without one the system shortcuts for cut, copy, paste, undo and select-all
/// are not bound at all. In a webview that means ⌘C does nothing in a transcript and ⌘Z does
/// nothing in a note: the app looked like it had lost the user's typing when it had simply never
/// been given the standard menu that carries those commands. That is what `PredefinedMenuItem`
/// provides, and it is the reason this exists at all; the rest is navigation.
///
/// Everything else emits `summo://menu` with its own id, and the interface decides what that means
/// — the same shape as the tray's record item. The shell stays a shell: it does not know what a
/// library is, only that something called `library` was chosen.
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let w = menu_words(&chosen_language(app));

    let item =
        |id: &str, label: &str, accel: Option<&str>| MenuItem::with_id(app, id, label, true, accel);

    let file = Submenu::with_items(
        app,
        w.file,
        true,
        &[
            &item("new-note", w.new_note, Some("CmdOrCtrl+N"))?,
            &item("import", w.import, Some("CmdOrCtrl+O"))?,
            &item("record", w.record, Some("CmdOrCtrl+Shift+R"))?,
            &PredefinedMenuItem::separator(app)?,
            &item("vault", w.vault, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    // The whole reason for the menu bar. These are the system's own items: they carry the standard
    // accelerators and they work inside the webview's text fields, which nothing we could write
    // here would.
    let edit = Submenu::with_items(
        app,
        w.edit,
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let view = Submenu::with_items(
        app,
        w.view,
        true,
        &[
            &item("home", w.home, Some("CmdOrCtrl+1"))?,
            &item("library", w.library, Some("CmdOrCtrl+2"))?,
            &item("tasks", w.tasks, Some("CmdOrCtrl+3"))?,
            &item("analytics", w.analytics, Some("CmdOrCtrl+4"))?,
            &PredefinedMenuItem::separator(app)?,
            &item("search", w.search, Some("CmdOrCtrl+K"))?,
            &item("sidebar", w.sidebar, Some("CmdOrCtrl+B"))?,
            &item("settings", w.settings, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
            &PredefinedMenuItem::minimize(app, None)?,
        ],
    )?;

    let help = Submenu::with_items(
        app,
        w.help,
        true,
        &[
            &item("shortcuts", w.shortcuts, None)?,
            &item("docs", w.docs, None)?,
            &item("issue", w.issue, None)?,
        ],
    )?;

    let menu = Menu::with_items(app, &[&file, &edit, &view, &help])?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let id = event.id().as_ref().to_string();
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        // One event with the id in it, rather than one event per item. What each of them means is
        // the interface's business — it owns the router, the palette and the sidebar — and a shell
        // that knew would be a second copy of the app's navigation, drifting.
        let _ = app.emit("summo://menu", id);
    });
    Ok(())
}
