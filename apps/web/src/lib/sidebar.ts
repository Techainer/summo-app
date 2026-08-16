/**
 * Whether the navigation column was left open.
 *
 * The column can be collapsed, and collapsing it was forgotten the moment the window reloaded — so
 * somebody who wanted the width back had to take it back on every launch, several times a day. The
 * theme has been remembered since it existed; this is the same promise for the other thing on this
 * screen a person deliberately sets.
 *
 * `localStorage` rather than the daemon's settings, for the reason the theme gives: it is a property
 * of this window, not of the vault, and one machine's narrow laptop should not fold the column on
 * the desktop beside it.
 *
 * Only the wide layout is remembered. The phone's sidebar is a sheet over the content, and a sheet
 * that reopened itself on every launch would be an app that starts with its menu covering it.
 */

export const STORAGE_KEY = "summo.sidebar";

/** Open unless the user last closed it. */
export function wasOpen(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) !== "closed";
  } catch {
    // Private browsing, a locked-down webview, a policy that blocks storage. A sidebar is not
    // worth an exception thrown during the first render.
    return true;
  }
}

export function remember(open: boolean): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, open ? "open" : "closed");
  } catch {
    // It still works for this session, which is better than not working at all.
  }
}
