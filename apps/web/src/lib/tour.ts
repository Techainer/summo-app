/**
 * What the first-run tour is about, and whether it has been seen.
 *
 * Apart from the component that draws it so the component file exports a component and nothing
 * else — `FirstRun` and the tour itself both need `seen()`, and a module that exports a hook, a
 * constant and a screen is a module every consumer reloads for the wrong reasons.
 */

const STORAGE_KEY = "summo.tour";

/** The four things a new user has to know, in the order they will need them. */
export const STEPS = ["record", "summary", "tasks", "ask"] as const;
export type Step = (typeof STEPS)[number];

/** Whether the tour has been seen. Local, like the language choice: it is about this screen. */
export function seen(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "done";
  } catch {
    return true; // Storage locked down: better to skip a tour than to show it every launch.
  }
}

export function markSeen(): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, "done");
  } catch {
    // Nothing to do; the tour will offer itself again next launch.
  }
}
