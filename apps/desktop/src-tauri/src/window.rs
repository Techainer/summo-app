//! Getting the window out of the way.
//!
//! The interface has had a compact bar for months — the record button, a meter and the last line
//! heard — and pressing it changed the *web layout* and nothing else. The window stayed the size of
//! an app you are working in, behind whatever you were actually doing, which is the opposite of
//! what somebody asks for when they shrink a recorder: they are about to watch a film, sit in a
//! call, or read something, and they want the words to keep arriving in the corner of the screen.
//!
//! So the bar now moves the window as well:
//!
//! - **Compact** — a strip, always on top, wherever it was last dragged. The daemon never stopped,
//!   so the transcript keeps coming.
//! - **Overlay** — the same strip with the window transparent and the background painted out, for
//!   watching something with live subtitles over it.
//!
//! Both remember what the window was, so leaving compact puts it back where the person left it
//! rather than in the middle of the screen at a default size. Both are no-ops in a browser, where
//! `window.__TAURI_INTERNALS__` does not exist and the interface falls back to what it always did.
//!
//! One window, resized. Not a second window: a separate compact window would need its own webview,
//! its own connection to the daemon and its own copy of the transcript, and the two would disagree
//! the first time one of them missed an event. The capability file has named a `compact` window
//! since before any of this existed — a permission for a window nobody ever created — and this is
//! the answer to it, rather than the window it was waiting for.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};

/// The strip's size, in logical pixels.
///
/// Wide enough for the button, a meter and a line of text at a readable size; short enough that it
/// is a strip rather than a small window. Matches what `RootLayout` draws in compact mode.
const STRIP: (f64, f64) = (560.0, 68.0);

/// What the window was before it was shrunk, so it can be put back.
#[derive(Default)]
pub struct Restore(Mutex<Option<Previous>>);

struct Previous {
    size: PhysicalSize<u32>,
    position: PhysicalPosition<i32>,
}

/// What the interface asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// The app, as it opens.
    Full,
    /// A strip on top of everything.
    Compact,
    /// A strip on top of everything, with no background of its own.
    Overlay,
}

fn main_window(app: &tauri::AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

/// Resize, float and restore the window.
///
/// Returns `Ok(())` and does nothing at all when there is no window — the same call from a browser
/// tab is not a failure, it is a call with nothing to act on.
#[tauri::command]
pub fn set_shape(app: tauri::AppHandle, restore: State<'_, Restore>, shape: Shape) -> Result<(), String> {
    let Some(window) = main_window(&app) else {
        return Ok(());
    };

    let mut previous = restore.0.lock().map_err(|e| e.to_string())?;

    match shape {
        Shape::Full => {
            // Put it back exactly, then let it be an ordinary window again. Order matters: a
            // window that stops floating before it is resized flashes behind whatever it was over.
            if let Some(was) = previous.take() {
                let _ = window.set_size(was.size);
                let _ = window.set_position(was.position);
            }
            window.set_always_on_top(false).map_err(|e| e.to_string())?;
            window.set_ignore_cursor_events(false).map_err(|e| e.to_string())?;
        }
        Shape::Compact | Shape::Overlay => {
            // Remembered once. Pressing compact twice must not record the strip as the thing to
            // restore to, which is how an app ends up permanently small.
            if previous.is_none() {
                previous.replace(Previous {
                    size: window.inner_size().map_err(|e| e.to_string())?,
                    position: window
                        .outer_position()
                        .map_err(|e| e.to_string())?,
                });
            }
            window.set_always_on_top(true).map_err(|e| e.to_string())?;
            window
                .set_size(LogicalSize::new(STRIP.0, STRIP.1))
                .map_err(|e| e.to_string())?;
            // Clicks still land on the strip. `ignore_cursor_events` would make the whole window
            // transparent to the mouse, including the stop button — which is the one control
            // somebody in overlay mode needs to reach.
            window.set_ignore_cursor_events(false).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Whether this build can float a window at all.
///
/// The interface asks before offering the control, so a browser tab does not get a button that
/// silently does nothing.
#[tauri::command]
#[must_use]
pub fn can_float() -> bool {
    true
}
