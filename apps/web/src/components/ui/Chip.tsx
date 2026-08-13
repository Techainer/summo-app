import { motion } from "motion/react";
import type { ReactNode } from "react";

import { cn } from "../../lib/cn";
import { SNAPPY } from "../../lib/motion";

/**
 * A small labelled thing: a tag, a filter, a person, a count.
 *
 * The app had six spellings of this — `rounded-full border px-2.5 py-1 text-meta`, `rounded-full
 * px-2 py-0.5 text-micro`, `rounded-lg px-2.5 py-0.5 font-mono text-micro` — and they sat next to
 * each other in the same rows. Two heights of pill in one line is the sort of thing that reads as
 * sloppiness without anyone being able to point at it.
 *
 * `on` is the selected state rather than a separate component, because a filter chip and a tag chip
 * differ only in whether the user can turn them off, and having one component means a tag can be
 * made clickable later without redrawing it.
 */
export function Chip({
  children,
  on = false,
  tone = "neutral",
  count,
  onClick,
  className,
  title,
}: {
  children: ReactNode;
  on?: boolean;
  tone?: "neutral" | "accent" | "ai";
  /** A number after the label, in tabular figures so a column of chips does not jitter. */
  count?: number;
  onClick?: () => void;
  className?: string;
  title?: string;
}) {
  const shell = cn(
    "text-meta inline-flex items-center gap-1.5 rounded-[var(--radius-pill)] border px-2.5 py-1 transition-colors",
    on
      ? tone === "ai"
        ? "border-ai/40 bg-ai-soft text-ai font-medium"
        : "border-accent/40 bg-accent-soft text-accent font-medium"
      : "border-line bg-bg-soft text-fg-dim",
    onClick && !on && "hover:border-line-strong hover:text-fg",
    className,
  );

  const inner = (
    <>
      {children}
      {count !== undefined && <span className="tabular text-fg-faint">{count}</span>}
    </>
  );

  if (!onClick) {
    return (
      <span className={shell} title={title}>
        {inner}
      </span>
    );
  }

  return (
    <motion.button
      type="button"
      onClick={onClick}
      aria-pressed={on}
      title={title}
      whileTap={{ scale: 0.96 }}
      transition={SNAPPY}
      className={shell}
    >
      {inner}
    </motion.button>
  );
}

/**
 * The label above a group inside a card or a column.
 *
 * Small, upper-case, wide-tracked, and quiet. Written out in eleven places with three different
 * sizes before this existed.
 */
export function SectionTitle({
  children,
  count,
  className,
}: {
  children: ReactNode;
  count?: number;
  className?: string;
}) {
  return (
    <h3
      className={cn("text-fg-faint text-micro font-semibold tracking-wider uppercase", className)}
    >
      {children}
      {count !== undefined && <span className="tabular ms-1.5 font-normal">{count}</span>}
    </h3>
  );
}
