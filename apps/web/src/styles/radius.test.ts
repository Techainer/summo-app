import { describe, expect, it } from "vitest";

import { callSites } from "./call-sites";

/**
 * One question, five answers, and a test so it stays five.
 *
 * "How round is a box" had six answers in a system that defined three. Counted across `src`:
 * `rounded-lg` 55 times, `rounded-md` 10, `rounded-2xl` 5, `rounded-xl` 4 — Tailwind's own scale,
 * sitting near but never on the theme's radii — against 77 uses of the real token spelled the long
 * way. Nobody chose that; it is what happens when a token exists and reaching past it is one
 * character shorter.
 *
 * So the tokens are `inline`, `control`, `card`, `panel`, `pill`, and this fails the build on
 * anything else. A rule is only a rule if something checks it — `theme.test.ts` next door exists
 * for the same reason, guarding the light/dark pairs against drift.
 *
 * This test shipped a release ago matching `rounded-`, and therefore said nothing at all about the
 * twelve call sites writing bare `rounded` — Tailwind's 4px, the exact class of thing it exists to
 * catch, sitting in nine files while the token block claimed it could not come back. The pattern
 * below anchors on the word. A guard with a hole in it is worse than no guard, because the hole is
 * invisible and the claim is not.
 *
 * What is deliberately still allowed, because each is a different question:
 *
 * - `rounded-full` — "as round as it can be", which is a shape rather than a size. It is what
 *   `--radius-pill` means for a box whose height is not known here.
 * - `rounded-none` and the directional `rounded-{t,b,l,r}-none` — removing a corner, not sizing it.
 * - a percentage, `rounded-[42%]` — an organic blob in `Spot`, which is not a box with corners.
 * - a token spelled out, `rounded-t-[var(--radius-panel)]` — a sheet rounding only its top edge.
 */
const ALLOWED = [
  /^rounded-(inline|control|card|panel|pill|full|none)$/,
  /^rounded-[tblrse]{1,2}-(inline|control|card|panel|pill|full|none)$/,
  /^rounded-[tblrse]{1,2}?-?\[var\(--radius-(inline|control|card|panel|pill)\)\]$/,
  /^rounded-\[\d+%\]$/,
];

/**
 * Anchored on the word, so bare `rounded` is a finding rather than a blind spot.
 *
 * `\brounded-[\w…]*` matched the hyphen before anything could follow it, which meant `rounded` on
 * its own never entered the loop. Matching the word and letting the suffix be optional is what
 * makes the two cases the same case.
 */
const RADIUS = /\brounded(-[\w[\]()%-]*)?/g;

describe("border radius", () => {
  it("is always one of the five tokens", () => {
    const offenders: string[] = [];
    for (const { file, line, text } of callSites("src")) {
      for (const match of text.matchAll(RADIUS)) {
        const found = match[0];
        if (!ALLOWED.some((shape) => shape.test(found))) {
          offenders.push(`${file}:${line} ${found}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
