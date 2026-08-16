import type { Transition, Variants } from "motion/react";

/**
 * One vocabulary for movement.
 *
 * Animation in an app like this has a job: tell the user what just changed and where it came from.
 * A panel that fades in from nowhere is decoration; one that rises from the row you clicked is an
 * explanation. So these are named after what they are *for* rather than after how they look, and
 * everything on screen picks from the same short list — twelve components each inventing their own
 * duration is what makes an interface feel assembled rather than designed.
 *
 * ## Durations
 *
 * Nothing here is longer than 260 ms. Above roughly 300 ms an animation stops reading as motion and
 * starts reading as lag; below about 80 ms it may as well not be there. Anything the user triggered
 * and is waiting on — a panel opening, a screen changing — sits at the fast end, because they have
 * already decided and are now waiting for the app to catch up.
 *
 * ## Reduced motion
 *
 * Honoured globally, not per component: `<MotionConfig reducedMotion="user">` at the root means
 * every `motion` element drops transforms and keeps opacity, without a single `useReducedMotion`
 * call in a component. That matters beyond preference — vestibular disorders make large sliding
 * movement genuinely unpleasant, and an app that animates a full-screen panel across the viewport
 * is unusable for those people. See {@link REDUCED}.
 */

/** Movement the user asked for and is waiting on: a menu, a panel, a screen. */
export const SNAPPY: Transition = { duration: 0.18, ease: [0.22, 1, 0.36, 1] };

/** Something arriving on its own: a nudge, a translation landing under a line. */
export const GENTLE: Transition = { duration: 0.26, ease: [0.22, 1, 0.36, 1] };

/**
 * Something being dragged, dropped, or following a finger.
 *
 * A spring rather than a duration, because a physical-feeling thing has to settle rather than stop.
 * Stiff and well damped: this is a UI control, not a bouncing ball, and overshoot on a task card
 * landing in a column reads as sloppiness.
 */
export const SPRING: Transition = {
  type: "spring",
  stiffness: 420,
  damping: 34,
  mass: 0.6,
};

/** A progress bar catching up to a number that moved. Slower on purpose: it is being read. */
export const METER: Transition = { duration: 0.3, ease: "easeOut" };

/**
 * What `reducedMotion="user"` actually does, written down because it is easy to assume it means
 * "no animation".
 *
 * Motion keeps opacity and drops transforms. That is the right split: a cross-fade still tells the
 * user something changed, while the sliding and scaling — the parts that cause discomfort — go
 * away. Nothing becomes invisible, and no interaction becomes unavailable.
 */
export const REDUCED = "user" as const;

/**
 * A list item appearing and leaving.
 *
 * Rises a little rather than sliding far: three pixels says "this is new" and does not make the
 * rest of the list look like it moved.
 */
export const listItem: Variants = {
  hidden: { opacity: 0, y: -3 },
  shown: { opacity: 1, y: 0 },
  gone: { opacity: 0, y: 0 },
};

/**
 * A panel or strip that expands from nothing.
 *
 * Animating `height: auto` needs the element to be measured, which Motion does — but only if the
 * child is not also animating its own height. Overflow stays hidden so the content does not spill
 * during the frames where the box is shorter than what is inside it.
 */
export const collapse: Variants = {
  hidden: { height: 0, opacity: 0 },
  shown: { height: "auto", opacity: 1 },
  gone: { height: 0, opacity: 0 },
};

/**
 * A screen replacing another.
 *
 * Deliberately small: four pixels and a fade. A full slide between routes turns every navigation
 * into a wait, and this app is one where people move between screens constantly — the library, a
 * meeting, back, tasks.
 */
export const screen: Variants = {
  hidden: { opacity: 0, y: 4 },
  shown: { opacity: 1, y: 0 },
  gone: { opacity: 0, y: -4 },
};

/**
 * How long the whole list may take to assemble, however many rows it has.
 *
 * The step used to be a constant, and a constant step is a delay that grows with the list: a vault
 * of a thousand meetings staggered at 15 ms put the last row 15 seconds behind the first, and the
 * rows start at `opacity: 0`. Measured on a seeded vault, the eight hundredth row was still
 * invisible ten seconds after the library opened, and the list took half a minute to finish
 * appearing. Nobody had opened the app on a vault that size.
 */
const ASSEMBLY_S = 0.4;

/**
 * Stagger a list so it assembles rather than appearing all at once.
 *
 * Capped twice: past about eight items the step shortens, and past a few dozen the *total* is what
 * is held to `ASSEMBLY_S` — a stagger is a hint that the list is arriving, not a queue the last row
 * waits in. `total` is how many are being rendered.
 */
export function stagger(total: number): Transition {
  const step = total > 8 ? 0.015 : 0.03;
  return { staggerChildren: Math.min(step, ASSEMBLY_S / Math.max(total, 1)) };
}

/**
 * How far to move something, given how far it *could* move.
 *
 * Used by anything that slides in from an edge. Long distances are slow to cross and read as
 * sluggish however short the duration is, so travel is capped — a sheet on a wide monitor should
 * not fly further than one on a laptop.
 */
export function travel(available: number): number {
  return Math.min(available, 320);
}
