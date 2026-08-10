import { useEffect, useState } from "react";

/** Below this the sidebar stops being a column and becomes a sheet. */
export const NARROW = 768;

/**
 * Whether a media query currently matches, kept in sync as the window changes.
 *
 * Reads the query once on mount rather than guessing a default, so the first paint on a phone is
 * not a desktop layout that collapses a frame later.
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() =>
    typeof window === "undefined" ? false : window.matchMedia(query).matches,
  );

  useEffect(() => {
    const list = window.matchMedia(query);
    const update = (event: MediaQueryListEvent) => setMatches(event.matches);
    setMatches(list.matches);
    list.addEventListener("change", update);
    return () => list.removeEventListener("change", update);
  }, [query]);

  return matches;
}

export function useIsNarrow(): boolean {
  return useMediaQuery(`(max-width: ${NARROW - 1}px)`);
}
