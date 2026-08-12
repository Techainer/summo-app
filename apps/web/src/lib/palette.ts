import type { SearchHit } from "./library";

/**
 * What ⌘K can find.
 *
 * Two kinds of result, deliberately in one list. **Places** are the eleven destinations the sidebar
 * holds, and **things** are what is in the vault. Splitting them into two searches would mean a
 * user has to know which one they are looking for before they can look — and "tasks" is a screen,
 * a word in a note and the name of an agent's file, all at once.
 *
 * Places rank above things when the query is short. Typing two letters is almost always somebody
 * navigating; typing a sentence is somebody searching.
 */

export interface Place {
  kind: "place";
  /** The router path. */
  to: string;
  label: string;
  /** Words that should find it beyond its label, so `models` finds `Mô hình` in any interface language. */
  keywords: string[];
}

export interface Thing {
  kind: "thing";
  id: string;
  title: string;
  day: string;
  /** Whether it was recorded or typed, which decides where opening it goes. */
  entry: "meeting" | "note";
  /** The line the match was found on, if the search returned one. */
  excerpt?: string;
}

export type Result = Place | Thing;

/** Below this, a query is a navigation; above it, a search. */
export const NAVIGATION_LENGTH = 3;

/**
 * Fold a string for comparison: lowercase, and Vietnamese tone marks removed.
 *
 * Typing `mo hinh` has to find `Mô hình`. A Vietnamese speaker searching their own notes on a
 * keyboard without a Vietnamese layout is the normal case, not an edge one, and an exact-match
 * palette is one they would stop using on the first try.
 */
export function fold(value: string): string {
  return (
    value
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "")
      // `\u0111` rather than the letter itself: `d` with a stroke is the one Vietnamese character
      // NFD does not decompose, so it needs its own rule — and written literally it trips the guard
      // that keeps interface text out of components, which cannot tell a normalisation rule from a
      // label.
      .replace(/\u0111/gi, "d")
      .toLowerCase()
      .trim()
  );
}

/** Places whose label or keywords contain every word of the query. */
export function matchPlaces(places: Place[], query: string): Place[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return places;
  return places.filter((place) => {
    const haystack = fold([place.label, ...place.keywords].join(" "));
    return words.every((word) => haystack.includes(word));
  });
}

/** Vault hits as palette results. */
export function asThings(hits: SearchHit[]): Thing[] {
  return hits.map((hit) => ({
    kind: "thing",
    id: hit.meeting.id,
    title: hit.meeting.title,
    day: hit.meeting.day,
    entry: hit.meeting.kind,
    excerpt: hit.excerpts[0]?.text.trim(),
  }));
}

/**
 * The list as shown.
 *
 * Short query: places first, because two letters is somebody navigating. Long query: things first,
 * because a sentence is somebody searching and the screens they could have meant are still there
 * underneath rather than in the way.
 */
export function order(places: Place[], things: Thing[], query: string): Result[] {
  return query.trim().length < NAVIGATION_LENGTH ? [...places, ...things] : [...things, ...places];
}
