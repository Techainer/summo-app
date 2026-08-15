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
 * Kept in `localStorage` rather than in the daemon's settings, for the reason the language is:
 * it is a property of *this screen*, not of the vault, and it has to be readable before the
 * handshake completes or the first paint is in the wrong theme.
 */

export const STORAGE_KEY = "summo.theme";

/** `system` is the default and means "do not set the attribute at all". */
export type Scheme = "system" | "light" | "dark";

export const SCHEMES: Scheme[] = ["system", "light", "dark"];

function isScheme(value: string | null): value is Scheme {
  return value === "system" || value === "light" || value === "dark";
}

/** What the user last chose, or `system`. */
export function read(): Scheme {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return isScheme(stored) ? stored : "system";
  } catch {
    // Private browsing, a locked-down webview, a policy that blocks storage. A theme is not worth
    // an exception thrown at module load.
    return "system";
  }
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

/** Remember and apply, which is always wanted together. */
export function choose(scheme: Scheme): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, scheme);
  } catch {
    // Applied anyway: it works for this session, which is better than not working at all.
  }
  apply(scheme);
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
