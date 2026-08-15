/**
 * The half second before the app has an engine, and the case where it never gets one.
 *
 * Only the desktop and mobile shells reach this. In a browser the daemon serves the page and has
 * already written the handshake into it, so there is nothing to wait for.
 *
 * **No translated words.** This renders above `AppI18n`, which reads the interface language from
 * the daemon — the thing that is not up yet. So it says what it can say without a language: the
 * mark, and, when there is one, the failure in the daemon's own words. A sentence in the wrong
 * language would be worse than none, and a hardcoded Vietnamese one is the bug the tray menu
 * already had.
 */
import { cn } from "../../lib/cn";

export function Starting({ error }: { error: string | null }) {
  return (
    <div className="bg-bg text-fg grid h-full place-items-center p-8">
      <div className="flex max-w-md flex-col items-center gap-4 text-center">
        {/* The same three bars as the header, breathing while there is nothing to report — the
            animation is the whole message: something is happening and it has not stalled. It stops
            when there is a failure to read, because a thing still working away under an error is a
            lie about what is going on. The keyframes are in `theme.css` and honour
            `prefers-reduced-motion`. */}
        <span
          className={cn("starting-mark", error !== null && "starting-mark-stopped")}
          aria-hidden="true"
        >
          <i />
          <i />
          <i />
        </span>
        <p className="text-title font-semibold tracking-tight">Summo</p>
        {error !== null && (
          // `pre-wrap`: the daemon's output arrives with the line breaks it wrote, and a stack of
          // messages folded into one paragraph is unreadable exactly when it matters.
          <p className="border-rec/30 bg-rec-soft text-rec text-meta w-full rounded-[var(--radius-card)] border px-3 py-2 text-start whitespace-pre-wrap">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
