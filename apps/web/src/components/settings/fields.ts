/**
 * One vocabulary for a settings row.
 *
 * These were four string constants at the top of a 500-line file that drew every setting in the
 * app. They are here so the sections can be separate files and still look like one screen — a
 * second definition of "a label beside a control" is how two settings panels end up disagreeing
 * about how wide a label is.
 */

/** One row of a form: a fixed-width label beside its control. */
export const FIELD = "mt-4 flex items-center gap-[var(--settings-field-gap)] text-meta text-fg-dim";

export const LABEL = "w-[var(--settings-label)] shrink-0";

/** Controls are `Input`s and `Select`s that stretch; the field owns everything else about them. */
export const CONTROL = "flex-1";

/**
 * Anything that sits under a control rather than under its label.
 *
 * A hint is the common case and has `HINT` below, but a progress bar wants the same indent without
 * the type — and the one place that needed it wrote the indent out as `ml-[162px]`, which is how
 * the number got into three files. The indent is a token now, computed from the label width and
 * the row's gap, so it cannot disagree with the row it belongs to.
 */
export const INDENT = "ms-[var(--settings-indent)]";

// `SELECT` used to live here — "a native `<select>`, which cannot be an `Input` — same box, drawn
// by hand". It could, and the hand-drawn copy had drifted: no focus ring, and the native arrow
// still sizing the control itself, so a dropdown never matched the input above it. `ui/Select`
// draws both, and this constant is gone rather than left as a second way to do it.

/** The note under a field. Indented to line up with the control it explains. */
export const HINT = `mt-1.5 ${INDENT} text-micro leading-normal text-fg-faint`;
