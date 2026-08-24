/**
 * Light, dark, or whatever the machine says.
 *
 * `theme.css` has defined `:root[data-theme="light"]` and `:root[data-theme="dark"]` since the
 * palette was rebuilt, and nothing has ever set the attribute. Dark mode has therefore only ever
 * followed the operating system: somebody on a light laptop who wants a dark app has had no way to
 * say so, and somebody demoing at night on a dark OS has had no way to turn it off. Two blocks of
 * CSS, fully written, unreachable — the same shape as the highlight nobody could see and the
 * `--color-danger` nothing defined.
 *
 * Read from `localStorage` and written to both. The browser has to be the one that answers before
 * the first paint — consulting the daemon first would put a round trip in front of the first pixel,
 * and the pixel would be the wrong colour while it waited. But the choice is not a property of one
 * window: `interface.theme` has been in the settings file since it had a schema, nothing ever wrote
 * it, and so choosing dark mode told the window you were in and nothing else. A second window, a
 * second machine and a reinstall each opened on `system`.
 *
 * So: {@link read} for the paint, {@link remember} to record it in both places, and {@link adopt}
 * for the one visit where this browser has nothing saved and the vault does.
 *
 * What is on screen lives here too — see {@link active} — because three components used to keep
 * their own copy of the answer and none of them could hear the others.
 */

import { url } from "./library";

export const STORAGE_KEY = "summo.theme";

/** `system` is the default and means "do not set the attribute at all". */
export type Scheme = "system" | "light" | "dark";

export const SCHEMES: Scheme[] = ["system", "light", "dark"];

function isScheme(value: string | null): value is Scheme {
  return value === "system" || value === "light" || value === "dark";
}

/** What the user last chose, or `system`. */
export function read(): Scheme {
  return stored() ?? "system";
}

/**
 * What this browser has been told, or `null` for "nobody has said here".
 *
 * The distinction `read()` collapses, and the one {@link adopt} needs: `system` is a choice
 * somebody made, and an empty slot is not. Reading them as the same value is why a preference set
 * on one machine could never travel to another — every browser looked like it had already chosen.
 */
export function stored(): Scheme | null {
  try {
    const value = window.localStorage.getItem(STORAGE_KEY);
    return isScheme(value) ? value : null;
  } catch {
    // Private browsing, a locked-down webview, a policy that blocks storage. A theme is not worth
    // an exception thrown at module load.
    return null;
  }
}

/**
 * The scheme in force, and everyone who is showing it.
 *
 * Not `localStorage`, and that is the point of it existing. {@link adopt} deliberately does not
 * write — the vault's answer must stay the vault's — so a browser that has just adopted `dark`
 * still reads `null` from storage, and a control asking storage what to display would say
 * "Hệ thống" over a dark screen. This is what is *painted*, which is what a control showing the
 * theme is for.
 *
 * It also settles a bug that predates the vault: the header button and the segmented control in
 * Cài đặt → Chung are on screen at the same time and each held its own `useState`, so changing the
 * theme in one left the other displaying the theme from before.
 */
let current: Scheme | null = null;
const watchers = new Set<() => void>();

/** What is on screen now. Falls back to storage on the first call, before anything has been set. */
export function active(): Scheme {
  current ??= read();
  return current;
}

/** Told whenever {@link active} changes. Shaped for `useSyncExternalStore`. */
export function subscribe(watcher: () => void): () => void {
  watchers.add(watcher);
  return () => {
    watchers.delete(watcher);
  };
}

function announce(scheme: Scheme): void {
  if (current === scheme) return;
  current = scheme;
  for (const watcher of watchers) watcher();
}

/**
 * Put the choice on the document.
 *
 * `system` *removes* the attribute rather than writing the current OS value into it, so the page
 * keeps following the machine when the machine changes — a user who switches their laptop to dark
 * at sunset should not have to come back here.
 */
export function apply(scheme: Scheme): void {
  const root = document.documentElement;
  if (scheme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", scheme);
}

/**
 * Remember, apply, and tell the vault — the three things a choice means.
 *
 * The last one is new and is the reason the setting existed in the first place. `interface.theme`
 * has been in `settings.json` since it had a schema and nothing wrote it, so the choice reached the
 * window it was made in and stopped: a second window, a second machine and a reinstall all opened
 * on `system`.
 *
 * Fire-and-forget on purpose. A theme is already applied locally by the time this is sent, and a
 * daemon that is not answering must not make the switch fail — the worst case is that this browser
 * knows and the next one does not, which is where things stood before.
 */
export function remember(handshake: { port: number; token: string }, scheme: Scheme): void {
  choose(scheme);
  void fetch(url(handshake, "/settings/interface"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ theme: scheme }),
  }).catch(() => undefined);
}

/** Remember locally and apply, which is always wanted together. */
export function choose(scheme: Scheme): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, scheme);
  } catch {
    // Applied anyway: it works for this session, which is better than not working at all.
  }
  apply(scheme);
  announce(scheme);
}

/**
 * Take the daemon's choice, but only when this browser has none of its own.
 *
 * The vault holds the preference too, and until now nothing read it — so choosing dark mode told
 * the window you were in and nothing else. A second window, a second machine and a reinstall each
 * started from `system`.
 *
 * Only when the slot is empty, and that restriction is the whole design. The browser stays the
 * authority for what is painted *now*: consulting the daemon before the first pixel would put a
 * round trip in front of it, and letting the daemon win afterwards would undo a choice somebody
 * had just made in this window. So this fires once per browser, on the visit that has nothing
 * saved, which is exactly the visit that should learn.
 *
 * Returns what it applied, or `null` when it left things alone.
 */
export function adopt(theme: string | undefined): Scheme | null {
  if (stored() !== null) return null;
  if (!isScheme(theme ?? null)) return null;
  const scheme = theme as Scheme;
  // Applied, not chosen: writing it to `localStorage` here would turn "the vault says dark" into
  // "this browser has decided dark", and the next change made elsewhere would stop arriving.
  apply(scheme);
  // Announced all the same. The screen has changed colour, so a control that says otherwise is
  // simply wrong — and this arrives after first paint, when those controls are already mounted.
  announce(scheme);
  return scheme;
}

/**
 * What the page is actually painting, following the OS when the choice is `system`.
 *
 * Optional call: `matchMedia` is missing in jsdom and in a few embedded webviews, and a theme label
 * is not worth a `TypeError` on a screen that would otherwise work. Light is the right guess when
 * nobody will say — it is what the stylesheet defaults to.
 */
export function resolved(scheme: Scheme): "light" | "dark" {
  if (scheme !== "system") return scheme;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}
