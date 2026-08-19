/**
 * The two things the interface needs from the app shell around it.
 *
 * In a browser the daemon serves this page and writes the handshake into the document, and the
 * record button is on screen. In the desktop and mobile apps neither is true: the page comes from
 * the bundle over a custom scheme, so it has no address for the daemon, and the tray icon and the
 * global shortcut are outside the window entirely.
 *
 * Both shells answer the same command and emit the same events, so this module has no idea which
 * one it is talking to.
 *
 * ## What was wrong
 *
 * Nothing here existed. The desktop shell emitted `summo://toggle-record` and the interface
 * listened for a DOM event of a different name; the mobile shell exposed a handshake command
 * nothing ever called. Both apps fell back to `{ port: 8710, token: "" }` — the development
 * default — and so connected to nothing at all.
 */

import type { Handshake } from "./engine";

/** How often to ask the shell whether the engine is up yet. */
const POLL_MS = 150;
/**
 * How long before the interface stops asking.
 *
 * Longer than the desktop shell's own 30 s, so that when the daemon does fail the message the user
 * sees is the daemon's — "port 8710 is in use" — rather than this module's "took too long".
 */
const GIVE_UP_MS = 40_000;

/** The command's answer. `starting` is the only one worth asking again after. */
type Status =
  | { status: "starting" }
  | { status: "ready"; port: number; token: string }
  | { status: "failed"; error: string };

/**
 * Whether there is a Tauri shell around this page.
 *
 * The internals object, not `window.__TAURI__`, which only exists when `withGlobalTauri` is on —
 * it is off here, because a global that can run commands is worth having only if something needs
 * it from the console.
 */
export function inShell(): boolean {
  return Boolean((globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
}

/**
 * What shape the window should be.
 *
 * `full` is the app. `compact` is the strip: short, wide, and above whatever the person is actually
 * doing — a call, a film, a document. `overlay` is the same strip with no background of its own,
 * for putting subtitles over something.
 */
export type Shape = "full" | "compact" | "overlay";

/**
 * Ask the shell to resize and float the window.
 *
 * Silently does nothing in a browser, which is the honest behaviour: a tab cannot float above other
 * windows, and the compact layout is still worth having there — it is the same strip, in a tab the
 * user can park wherever their window manager allows.
 *
 * Errors are swallowed on purpose. This is a nicety on top of a recording that is already running;
 * a window manager refusing to float a window must not put a banner over a meeting.
 */
export async function setShape(shape: Shape): Promise<void> {
  if (!inShell()) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("set_shape", { shape });
  } catch {
    // A shell too old to know this command, or a platform that refuses. The layout still changed.
  }
}

/**
 * Whether this is a Mac, which decides two visible things.
 *
 * The modifier a shortcut is written with — `⌘K` on a Mac and `Ctrl+K` everywhere else, and a sheet
 * that shows the wrong one is worse than no sheet — and where the menu lives. macOS puts a menu bar
 * at the top of the *screen*, so the native one is the right one and the window should not draw a
 * second. Windows and Linux put it in the window, and this window has `decorations: false`: there
 * is no frame for the system to hang a menu on, so the app draws its own.
 *
 * `userAgentData` first because `navigator.platform` is deprecated and lies under emulation; the
 * old field is the fallback that still answers on every webview this ships in.
 */
export function isMac(): boolean {
  const data = (navigator as { userAgentData?: { platform?: string } }).userAgentData;
  const platform = data?.platform ?? navigator.platform ?? "";
  return /mac/i.test(platform);
}

/**
 * Where the daemon is, once the shell has one to give.
 *
 * Polls rather than waiting on the ready event alone, and for a reason that only shows on a fast
 * machine: the engine can be up before this module is imported, and a listener registered after
 * the event has already fired waits forever. Asking is idempotent; listening is not.
 *
 * Rejects with whatever the shell said went wrong. That sentence comes from the daemon's own
 * output — a vault that cannot be read, a port already taken — and is the only description of the
 * failure the user will ever get.
 *
 * The two intervals are arguments only so that a test can run the whole wait in a few
 * milliseconds. Nothing in the app passes them, and a fake clock was the alternative: this
 * function's loop schedules its next sleep only after an answer arrives, which is precisely the
 * shape a fake clock cannot be stepped through without knowing how many answers are coming.
 */
export async function shellHandshake({
  pollMs = POLL_MS,
  giveUpMs = GIVE_UP_MS,
}: { pollMs?: number; giveUpMs?: number } = {}): Promise<Handshake> {
  const { invoke } = await import("@tauri-apps/api/core");
  const until = Date.now() + giveUpMs;

  for (;;) {
    let status: Status;
    try {
      status = await invoke<Status>("engine_handshake");
    } catch (error) {
      // The command itself is unreachable — an older shell, or a build where the handler was not
      // registered. Retrying cannot fix that, and pretending otherwise costs the user 40 seconds.
      throw new Error(`this app's shell cannot reach its engine: ${String(error)}`, {
        cause: error,
      });
    }

    if (status.status === "ready") return { port: status.port, token: status.token };
    if (status.status === "failed") throw new Error(status.error);
    if (Date.now() > until) throw new Error("the engine did not start");
    await new Promise((resume) => setTimeout(resume, pollMs));
  }
}

/**
 * Let the tray icon and the global shortcut reach the record button.
 *
 * The shell emits a Tauri event, which is not a DOM event and does not reach an
 * `addEventListener`. This is the one line of translation between them, and without it both of the
 * shell's two features did nothing: pressing ⌘⇧R started a recording in a process that had no way
 * to tell the window about it.
 *
 * Returns a function that stops listening, or `null` outside a shell.
 */
export async function bridgeShellEvents(): Promise<(() => void) | null> {
  if (!inShell()) return null;
  const { listen } = await import("@tauri-apps/api/event");
  const stops = await Promise.all([
    listen("summo://toggle-record", () => {
      window.dispatchEvent(new CustomEvent("summo:toggle-record"));
    }),
    // The menu bar. The shell knows only that something called `library` was chosen; what that
    // means is decided here, where the router and the palette are. See `build_menu` for why the
    // menu exists at all — an app with no menu bar on macOS has no working ⌘C.
    listen<string>("summo://menu", (event) => {
      window.dispatchEvent(new CustomEvent("summo:menu", { detail: event.payload }));
    }),
  ]);
  return () => stops.forEach((stop) => stop());
}
