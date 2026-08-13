import { motion } from "motion/react";
import type { ReactNode } from "react";

import { cn } from "../../lib/cn";
import { GENTLE } from "../../lib/motion";

/**
 * One frame for every screen.
 *
 * Nine screens had six container widths and four paddings between them — `max-w-3xl p-5`,
 * `max-w-4xl space-y-4 p-5`, `px-6 py-8`, `p-5` with no limit at all. Nothing about that is visible
 * one screen at a time, and all of it is visible when you move between them: the heading jumps left,
 * the content column changes width, the gap under the title changes. It reads as several apps
 * stitched together, which is exactly what the complaint about the layout was.
 *
 * So the frame is decided once, here:
 *
 * - **One column width.** `max-w-6xl` — wide enough for a two-column dashboard at 1280 and still a
 *   readable measure, because nothing inside it is a paragraph running the full width.
 * - **One padding rhythm.** 20px on a phone, 32px from `sm` up. Vertical matches horizontal.
 * - **One gap.** 24px between sections, everywhere. A screen that needs a different gap between two
 *   things needs a card, not a different gap.
 * - **One entrance.** Fade and four pixels of rise, once, on mount. Long enough to be seen, short
 *   enough that a person moving quickly through screens never waits for it.
 *
 * A screen that manages its own scrolling — the note editor, the transcript, the kanban board —
 * passes `fill`, which drops the vertical padding to a frame that fills the pane and lets the
 * screen decide what scrolls inside it.
 */
export function Page({
  title,
  subtitle,
  eyebrow,
  actions,
  aside,
  fill = false,
  width = "wide",
  children,
  className,
  ...rest
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  /** A quiet line above the title: a greeting, a count, a breadcrumb. */
  eyebrow?: ReactNode;
  /** Buttons, on the right of the title on a wide screen and under it on a narrow one. */
  actions?: ReactNode;
  /** Numbers or chips that belong to the heading rather than to the content. */
  aside?: ReactNode;
  fill?: boolean;
  /**
   * `narrow` for a form or a single column of prose, `wide` for a dashboard.
   *
   * Two widths, not five. A settings form at 1152px is a row of labels with a metre of nothing
   * beside them; a dashboard at 672px is a single column of cards on a wide screen. Every screen is
   * one of those two things.
   */
  width?: "wide" | "narrow";
  children: ReactNode;
  className?: string;
} & { "data-testid"?: string }) {
  return (
    <div className={cn(fill && "h-full min-h-0", "relative")} {...rest}>
      <div
        className={cn(
          // Left-aligned, not centred. `mx-auto` on a narrow column means the heading of a form
          // screen starts a hundred pixels further right than the heading of a dashboard, and the
          // whole interface shifts sideways as you move between them. Capped by a maximum width so
          // a very wide monitor does not stretch a form across it, but the first character of every
          // screen sits on the same vertical line.
          "flex w-full flex-col gap-6 px-5 sm:px-8",
          width === "wide" ? "max-w-6xl" : "max-w-3xl",
          fill ? "h-full min-h-0 py-5 sm:py-6" : "py-6 sm:py-8",
          className,
        )}
      >
        {(title || actions || aside) && (
          <motion.div
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            transition={GENTLE}
            // A `div`, not a `header`. `<header>` inside `<main>` is valid HTML and is not a
            // banner landmark, but it is still a `header` element, and every browser suite that
            // reaches for "the app's header" by tag name now finds two. The heading block needs no
            // element of its own to be correct — the `h1` inside it is what carries the meaning.
            className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3"
          >
            <div className="min-w-0">
              {eyebrow && <p className="text-fg-faint text-meta">{eyebrow}</p>}
              {title && <h1 className="text-display font-semibold">{title}</h1>}
              {subtitle && <p className="text-fg-dim text-meta mt-1.5 max-w-2xl">{subtitle}</p>}
            </div>
            {aside}
            {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
          </motion.div>
        )}

        {/* The content gets its own fade so it arrives just after the heading rather than with it.
            Sixty milliseconds is below the threshold where anybody would call it a sequence, and
            above the one where the screen reads as a single flat image appearing. */}
        <motion.div
          initial={{ opacity: 0, y: 4 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ ...GENTLE, delay: 0.06 }}
          className={cn("flex flex-col gap-6", fill && "min-h-0 flex-1")}
        >
          {children}
        </motion.div>
      </div>
    </div>
  );
}

/**
 * The soft light behind a heading.
 *
 * Absolute and `pointer-events-none`, drawn under the frame's own content, clipped to the top of
 * the screen. Only screens that open a section of the app get one — a wash on every screen is
 * wallpaper, and wallpaper stops meaning anything.
 */
export function PageGlow({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute inset-x-0 top-0 h-72 bg-[image:var(--gradient-page)]",
        className,
      )}
    />
  );
}
