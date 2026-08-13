import { useCallback, useSyncExternalStore } from "react";

/** Below this the sidebar stops being a column and becomes a sheet. */
export const NARROW = 768;

/**
 * Whether a media query currently matches, kept in sync as the window changes.
 *
 * `useSyncExternalStore` rather than state-plus-effect. The media query *is* an external store —
 * something outside React that changes on its own and has to be read consistently — and this is the
 * hook React added for exactly that. The version this replaces initialised state from
 * `matchMedia`, then set it again inside an effect, which meant a synchronous re-render on every
 * mount for a value that had not changed, and a window resized between render and effect could be
 * read twice with different answers. It is also why `react-hooks/set-state-in-effect` fired here:
 * that one was a real cascade, not a false positive.
 */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (onChange: () => void) => {
      const list = window.matchMedia(query);
      list.addEventListener("change", onChange);
      return () => list.removeEventListener("change", onChange);
    },
    [query],
  );

  return useSyncExternalStore(
    subscribe,
    () => window.matchMedia(query).matches,
    // The server has no window and no viewport. Desktop is the honest guess for a pre-render, and
    // the first client read corrects it before paint.
    () => false,
  );
}

export function useIsNarrow(): boolean {
  return useMediaQuery(`(max-width: ${NARROW - 1}px)`);
}
