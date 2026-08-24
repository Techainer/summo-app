import { useSyncExternalStore } from "react";

import { active, subscribe, type Scheme } from "./theme";

/**
 * The theme every control shows, from one place.
 *
 * Each of the three controls that displays the scheme — the header button, the segmented control in
 * Cài đặt → Chung, and the ⌘K entries — used to open with `useState(read())` and update only itself.
 * The header and the settings screen are on screen together, so changing the theme in one left the
 * other showing the theme from before; and neither could hear {@link adopt}, which is how the
 * vault's saved choice arrives on a browser that has none, so that visit painted dark under a
 * control still reading "Hệ thống".
 *
 * `useSyncExternalStore` rather than an effect and a piece of state: the value is already outside
 * React, `active()` returns a string so the snapshot compares by value, and the server snapshot is
 * the same call because nothing here renders on a server.
 */
export function useScheme(): Scheme {
  return useSyncExternalStore(subscribe, active, active);
}
