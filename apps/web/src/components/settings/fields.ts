/**
 * One vocabulary for a settings row.
 *
 * These were four string constants at the top of a 500-line file that drew every setting in the
 * app. They are here so the sections can be separate files and still look like one screen — a
 * second definition of "a label beside a control" is how two settings panels end up disagreeing
 * about how wide a label is.
 */

/** One row of a form: a fixed-width label beside its control. */
export const FIELD = "mt-3.5 flex items-center gap-3 text-meta text-fg-dim";

export const LABEL = "w-[150px] shrink-0";

/** Controls are `Input`s that stretch; the field owns everything else about them. */
export const CONTROL = "flex-1";

/** A native `<select>`, which cannot be an `Input` — same box, drawn by hand. */
export const SELECT =
  "h-9 flex-1 rounded-[var(--radius-card)] border border-line bg-bg-soft px-3 text-sm text-fg" +
  " transition-colors hover:border-line-strong focus-visible:border-accent focus:outline-none";

/** The note under a field. Indented to line up with the control it explains. */
export const HINT = "mt-1.5 ml-[162px] text-micro leading-normal text-fg-faint";
