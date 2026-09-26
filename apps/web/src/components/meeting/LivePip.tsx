import { Minimize2, Square } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { askCompact, inShell } from "../../lib/shell";

import { Button } from "../ui";
import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { canFloat, openFloatingWindow } from "../../lib/float";

/**
 * The meeting, small, and out of the way.
 *
 * A live session fills the screen: notes on the left, transcript on the right, controls above both.
 * That is the right shape for a meeting somebody is *in*. It is the wrong shape for the other half
 * of what this app is for — sitting in a call held in a language you half-follow, or watching a
 * talk, where the window you need to see is somebody else's and all you want from Summo is the
 * sentence just said and what it means.
 *
 * So: the last line, its translation, and the two controls worth having at that size. Nothing else.
 * No waveform, no notes, no transcript to scroll — a panel you read rather than use.
 *
 * **A real window where the browser has one.** Document Picture-in-Picture opens an operating
 * system window that stays above the others, which is the whole point: a panel pinned inside a page
 * disappears the moment somebody clicks the call they are in. Where the API is absent — every
 * WebKit-based webview today, which is the desktop app on macOS and Linux — it pins to the corner
 * of this page instead, and says which of the two it did. A control that silently does something
 * different from what it says is worse than one that is not there.
 */
export function LivePip() {
  const t = useT();
  const { transcript, stop } = useEngine();
  const [floating, setFloating] = useState<Document | null>(null);
  const [pinned, setPinned] = useState(false);
  const closing = useRef<(() => void) | null>(null);

  // The last line with words in it, and the partial that has not settled yet.
  //
  // Newest-last, so this reads from the end. A partial is deliberately preferred when there is one:
  // at this size the interesting line is the one being spoken, and waiting for it to finalise is
  // the difference between a caption and a log.
  const segments = transcript.segments;
  const last = segments.at(-1) ?? null;

  const close = useCallback(() => {
    closing.current?.();
    closing.current = null;
    setFloating(null);
    setPinned(false);
  }, []);

  const open = useCallback(async () => {
    if (floating || pinned) {
      close();
      return;
    }
    if (!canFloat()) {
      setPinned(true);
      return;
    }
    const opened = await openFloatingWindow({ width: 460, height: 220 });
    if (!opened) {
      // The API exists and refused — a second window already open, or a gesture the browser did not
      // count. Pinning is still useful, so do that rather than nothing.
      setPinned(true);
      return;
    }
    closing.current = opened.close;
    opened.onClose(() => {
      closing.current = null;
      setFloating(null);
    });
    setFloating(opened.document);
  }, [floating, pinned, close]);

  // A window that outlives the recording is a window showing a meeting that ended.
  useEffect(() => () => closing.current?.(), []);

  const panel = (
    <div className="bg-bg text-fg flex h-full flex-col gap-2 p-3" data-testid="live-pip">
      <div className="flex items-center gap-2">
        <span aria-hidden="true" className="relative flex size-2.5 shrink-0 items-center">
          <span className="bg-rec absolute inline-flex size-2.5 rounded-full" />
          <span className="bg-rec/60 absolute inline-flex size-2.5 rounded-full motion-safe:animate-ping" />
        </span>
        <p className="text-rec text-micro leading-none font-medium">{t("record.recording_now")}</p>
        <div className="ms-auto flex items-center gap-1.5">
          <Button size="sm" variant="ghost" onClick={close} aria-label={t("record.restore")}>
            <Minimize2 aria-hidden="true" className="size-3" />
          </Button>
          <Button size="sm" variant="danger" onClick={stop}>
            <Square aria-hidden="true" className="me-1.5 size-3" />
            {t("record.stop")}
          </Button>
        </div>
      </div>

      {/* The line, at a size somebody reads from across a desk rather than leans into. */}
      <div className="min-h-0 flex-1 overflow-hidden">
        {last ? (
          <>
            <p className="text-body leading-snug font-medium" data-testid="pip-original">
              {last.text}
            </p>
            {last.translations?.map((translation) => (
              <p
                key={translation.lang}
                lang={translation.lang}
                data-testid="pip-translation"
                className="text-fg-dim text-meta mt-1.5 leading-snug"
              >
                {translation.text}
              </p>
            ))}
          </>
        ) : (
          <p className="text-fg-faint text-meta">{t("record.listening")}</p>
        )}
      </div>
    </div>
  );

  return (
    <>
      <Button
        size="sm"
        variant="ghost"
        onClick={() => {
          // In the desktop app, minimising means the *window* shrinks: a strip that floats over
          // whatever you are actually doing, draggable anywhere, with a transparent mode for
          // putting the line over a film. That has existed since the shell did.
          //
          // This button opened a panel in the corner of the app instead — which is the fallback
          // written for browsers, where there is no window to shrink. Two features answering to
          // one word, and the button people press reached the lesser one.
          if (inShell()) {
            askCompact();
            return;
          }
          void open();
        }}
        className="shrink-0"
      >
        <Minimize2 aria-hidden="true" className="me-1.5 size-3" />
        {t("record.minimise")}
      </Button>

      {/* A real window. */}
      {floating && createPortal(panel, floating.body)}

      {/* Or a corner of this one, with the difference stated rather than hidden. */}
      {pinned && (
        <div className="border-line bg-bg-raised rounded-card fixed end-4 bottom-4 z-[var(--z-float)] w-[min(26rem,calc(100vw-2rem))] border shadow-[var(--shadow-pop)]">
          {panel}
          <p className="text-fg-faint text-micro border-line border-t px-3 py-2">
            {t("record.minimise_pinned")}
          </p>
        </div>
      )}
    </>
  );
}
