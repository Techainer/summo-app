import type { SearchHit } from "./library";

/**
 * What ⌘K can find.
 *
 * Three kinds of result, deliberately in one list. **Actions** are things you do, **places** are
 * the destinations the sidebar holds, and **things** are what is in the vault. Splitting them into
 * separate searches would mean a user has to know which one they are looking for before they can
 * look — and "ghi" is a verb, a screen and a word inside a note, all at once.
 *
 * Actions rank first. Somebody who typed a verb has told you what they want to happen, and making
 * them scroll past two screens with that word in the name to reach it is the palette failing at
 * the one thing it is for. Places then rank above things when the query is short: two letters is
 * almost always navigation, a sentence is a search.
 */

export interface Place {
  kind: "place";
  /** The router path. */
  to: string;
  label: string;
  /** Words that should find it beyond its label, so `models` finds `Mô hình` in any interface language. */
  keywords: string[];
}

/**
 * Something the palette does rather than somewhere it goes.
 *
 * Carries its own closure. The alternative — an id the caller switches on — puts the list of what
 * exists in one file and the list of what each one means in another, and the two drift the first
 * time somebody adds a command and forgets the second half.
 */
export interface Action {
  kind: "action";
  id: string;
  label: string;
  keywords: string[];
  run: () => void;
}

/** A place or an action: the half of the palette that is not the vault. */
export type Command = Place | Action;

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

export type Result = Command | Thing;

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

/** Commands whose label or keywords contain every word of the query. */
export function matchCommands<T extends { label: string; keywords: string[] }>(
  all: T[],
  query: string,
): T[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return all;
  return all.filter((one) => {
    const haystack = fold([one.label, ...one.keywords].join(" "));
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
 * Actions always first — a verb is an instruction, and there are never more than a handful of them
 * matching. Then: short query, places before things, because two letters is somebody navigating;
 * long query, things before places, because a sentence is somebody searching and the screens they
 * could have meant are still there underneath rather than in the way.
 */
export function order(commands: Command[], things: Thing[], query: string): Result[] {
  const actions = commands.filter((one): one is Action => one.kind === "action");
  const places = commands.filter((one): one is Place => one.kind === "place");
  return query.trim().length < NAVIGATION_LENGTH
    ? [...actions, ...places, ...things]
    : [...actions, ...things, ...places];
}
