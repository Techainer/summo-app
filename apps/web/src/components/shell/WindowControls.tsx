import { Minus, Square, Copy, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { useT } from "../../i18n/context";
import { inShell } from "../../lib/shell";

/**
 * Minimise, maximise and close, drawn by the app.
 *
 * The desktop window is created with `decorations: false` so that the header can be the title bar —
 * which is the look every reference in this product's design has, and which nothing was paying
 * for: with the system's title bar gone and no replacement, the window could not be minimised,
 * could not be maximised, and could only be closed through the tray icon. On Linux and Windows
 * there was no other way to put it away at all.
 *
 * Renders nothing in a browser, where the browser's own chrome is doing this job.
 *
 * The window API is loaded on demand rather than imported, so a browser build never pulls it into
 * the chunk every screen waits for.
 */
export function WindowControls() {
  const t = useT();
  const [maximised, setMaximised] = useState(false);
  const shell = inShell();

  useEffect(() => {
    if (!shell) return undefined;
    let live = true;
    let stop: (() => void) | null = null;

    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const window = getCurrentWindow();
      const read = async () => {
        const is = await window.isMaximized();
        if (live) setMaximised(is);
      };
      void read();
      // Dragging a window to the top edge maximises it without going through the button, and a
      // button that then offers to maximise an already-maximised window is a button that lies.
      const unlisten = await window.onResized(() => void read());
      if (live) stop = unlisten;
      else unlisten();
    })();

    return () => {
      live = false;
      stop?.();
    };
  }, [shell]);

  const act = useCallback(async (what: "minimise" | "toggle" | "close") => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const window = getCurrentWindow();
    if (what === "minimise") return window.minimize();
    if (what === "toggle") return window.toggleMaximize();
    // Close, not destroy: the shell turns this into hiding, because a recording has to survive
    // somebody tidying their desktop. Quitting is the tray's `Thoát`.
    return window.close();
  }, []);

  if (!shell) return null;

  return (
    <div className="ms-1 hidden items-center gap-0.5 sm:flex">
      <button
        type="button"
        onClick={() => void act("minimise")}
        aria-label={t("nav.minimise")}
        title={t("nav.minimise")}
        className="text-fg-faint hover:bg-bg-soft hover:text-fg rounded-lg px-2 py-1.5"
      >
        <Minus aria-hidden="true" className="size-4 stroke-[1.75]" />
      </button>
      <button
        type="button"
        onClick={() => void act("toggle")}
        aria-label={maximised ? t("nav.restore") : t("nav.maximise")}
        title={maximised ? t("nav.restore") : t("nav.maximise")}
        className="text-fg-faint hover:bg-bg-soft hover:text-fg rounded-lg px-2 py-1.5"
      >
        {maximised ? (
          <Copy aria-hidden="true" className="size-3.5 stroke-[1.75]" />
        ) : (
          <Square aria-hidden="true" className="size-3.5 stroke-[1.75]" />
        )}
      </button>
      <button
        type="button"
        onClick={() => void act("close")}
        aria-label={t("nav.close")}
        title={t("nav.close")}
        // The only control in the app that turns red on hover. Closing is the one action here a
        // person can take by accident and cannot undo by clicking again. The same pair as every
        // other red surface in the app, so it is the palette's red on the palette's tint rather
        // than white on a saturated fill that has to be checked separately in both schemes.
        className="text-fg-faint hover:bg-rec-soft hover:text-rec rounded-lg px-2 py-1.5"
      >
        <X aria-hidden="true" className="size-4 stroke-[1.75]" />
      </button>
    </div>
  );
}
