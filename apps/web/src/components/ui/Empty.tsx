import type { LucideIcon } from "lucide-react";
import { m } from "motion/react";
import { Suspense, lazy, type ReactNode } from "react";

import { cn } from "../../lib/cn";
import { GENTLE } from "../../lib/motion";
import { Spot } from "./Spot";
import type { StickerName } from "./Sticker";

/**
 * Fetched when a sticker is actually drawn, which is on an empty screen and nowhere else.
 *
 * Imported directly it landed in the entry chunk — the drawings, the player's loader and all — and
 * put the first load 0.8 kB over its budget for art that most sessions never see. The fallback while
 * it arrives is [`Spot`], which is what this component drew before stickers existed, so the wait
 * shows the old illustration rather than a hole.
 */
const Sticker = lazy(() => import("./Sticker").then((m) => ({ default: m.Sticker })));

/**
 * A screen with nothing on it yet.
 *
 * Empty was one grey sentence in the middle of a pane, or — on the task board — four bordered boxes
 * and six hundred pixels of nothing. Both read as *broken* rather than as *new*, which is the worst
 * possible first impression: the one moment every user passes through is the moment the app has
 * least to show them.
 *
 * Three things, in this order, and no more:
 *
 * - **A drawing** — see [`Spot`] — with the screen's icon at the centre of it. It gives the eye
 *   somewhere to land in a space that otherwise has no landmark, and it says which screen this is
 *   without repeating the heading. It moves, slowly, because a still empty screen reads as a screen
 *   that failed rather than one that is waiting for you.
 * - **What is true**, not what went wrong. "No meetings yet" — not "failed to load", which is a
 *   different state and deserves a different message.
 * - **What to do about it**, when there is something. An empty state with no way out is a dead end,
 *   and an empty state with a *button* is where a lot of people take their first action.
 *
 * `role="status"` so a screen reader is told the list came back empty rather than being left to
 * infer it from silence.
 */
export function Empty({
  icon: Icon,
  sticker,
  title,
  hint,
  action,
  className,
  /**
   * Take the whole pane and sit in the middle of it.
   *
   * For a screen whose *only* content is this. Top-aligned in a tall empty pane, an empty state
   * reads as a screen that failed to load the rest — five hundred pixels of nothing under two
   * lines of grey text is what "broken" looks like. Centred, the emptiness is the composition.
   */
  full = false,
}: {
  icon: LucideIcon;
  /**
   * A drawing with something happening in it, in place of the generic one.
   *
   * `Spot` says *which screen this is* — the screen's own icon, in a tinted blob. A sticker says
   * *how to feel about it*, which on the one screen a new user is most likely to judge the app by
   * is the more useful of the two. Where a sticker is given it replaces the spot rather than
   * sitting beside it: two illustrations stacked is a collage, not a composition.
   */
  sticker?: StickerName;
  title: string;
  hint?: string;
  action?: ReactNode;
  className?: string;
  full?: boolean;
}) {
  return (
    <m.div
      role="status"
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={GENTLE}
      className={cn(
        "flex flex-col items-center justify-center gap-3 px-6 text-center",
        // Uncapped on purpose, and it was tried the other way. Bounding this so it would sit closer
        // to a form above it made every screen where the empty state *is* the content fail
        // `e2e/density.mjs` — four hundred pixels of background under a message that had been
        // centred. The gap-after-a-form problem is real but it belongs to the caller: `full` means
        // "this is the whole screen", and the screen with a subscribe form above it should not be
        // asking for it. See `AgendaScreen`.
        full ? "h-full py-10" : "py-14",
        className,
      )}
    >
      {/* A drawing rather than a bare glyph: at 20px an outline icon on a flat background is a
          smudge, and this is the one screen in the app that has nothing else on it to look at. */}
      {sticker ? (
        <m.span
          className="mb-1 inline-flex"
          initial={{ scale: 0.85, opacity: 0 }}
          animate={{ scale: 1, opacity: 1 }}
          transition={{ ...GENTLE, delay: 0.05 }}
        >
          <Suspense fallback={<Spot icon={Icon} size={full ? 128 : 108} />}>
            <Sticker name={sticker} size={full ? 128 : 108} />
          </Suspense>
        </m.span>
      ) : (
        <Spot icon={Icon} size={full ? 104 : 84} className="mb-1" />
      )}
      <p className="text-fg text-body font-medium">{title}</p>
      {hint && <p className="text-fg-faint text-meta max-w-sm leading-relaxed">{hint}</p>}
      {action}
    </m.div>
  );
}

/**
 * The same idea inside a column that is only one of several.
 *
 * Smaller and quieter: four of these side by side on a task board must not each shout. No icon
 * disc, no action — the board's own controls are above it, and repeating them per column would be
 * four buttons that all do the same thing.
 */
export function EmptyColumn({ children }: { children: ReactNode }) {
  return (
    <p className="text-fg-faint text-micro px-2 py-6 text-center leading-relaxed">{children}</p>
  );
}
