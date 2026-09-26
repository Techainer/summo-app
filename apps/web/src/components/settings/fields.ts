/**
 * One vocabulary for a settings row.
 *
 * These were four string constants at the top of a 500-line file that drew every setting in the
 * app. They are here so the sections can be separate files and still look like one screen — a
 * second definition of "a label beside a control" is how two settings panels end up disagreeing
 * about how wide a label is.
 */

/**
 * One row of a form: a label beside its control, until there is not room for both.
 *
 * The label is a fixed 150px, which is what makes every panel line up. On a wide screen that is
 * most of the point; below one it is most of the width. At 768 pixels — a tablet held upright, a
 * laptop with another window beside it — the app sidebar takes 210, the settings rail takes
 * another 210, and what was left gave the control 78 pixels: `vi`, `auto`, `30`, `this-machine`.
 * Enough to see that something is set and not enough to read it or change it.
 *
 * So the row stacks before it squeezes. The alignment that the fixed label buys is worth having
 * exactly while there is room for it.
 */
export const FIELD =
  "mt-4 flex flex-col items-start gap-1 text-meta text-fg-dim lg:flex-row lg:items-center lg:gap-[var(--settings-field-gap)]";

export const LABEL = "shrink-0 lg:w-[var(--settings-label)]";

/**
 * Controls are `Input`s and `Select`s that stretch; the field owns everything else about them.
 *
 * `w-full` below `lg`, because the row is a column there and `flex-1` in a column grows the wrong
 * axis — it would stretch a text field's *height* and leave it as narrow as its content.
 */
export const CONTROL = "w-full lg:w-auto lg:flex-1";

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
export const HINT = `mt-1.5 text-micro leading-normal text-fg-faint lg:${INDENT}`;
