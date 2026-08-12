import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "../../lib/cn";

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
 * - **An icon**, dimmed and in a soft disc. It gives the eye somewhere to land in a space that
 *   otherwise has no landmark, and it says which screen this is without repeating the heading.
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
  title,
  hint,
  action,
  className,
}: {
  icon: LucideIcon;
  title: string;
  hint?: string;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div
      role="status"
      className={cn(
        "flex flex-col items-center justify-center gap-3 px-6 py-14 text-center",
        className,
      )}
    >
      <span className="bg-bg-soft ring-line grid size-12 place-items-center rounded-full ring-1">
        <Icon aria-hidden="true" className="text-fg-faint size-5 stroke-[1.5]" />
      </span>
      <p className="text-fg text-[15px] font-medium">{title}</p>
      {hint && <p className="text-fg-faint max-w-xs text-[13px] leading-relaxed">{hint}</p>}
      {action}
    </div>
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
    <p className="text-fg-faint px-2 py-6 text-center text-[12px] leading-relaxed">{children}</p>
  );
}
