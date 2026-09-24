import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * One question, four answers, and a test so it stays four.
 *
 * "How round is a box" had six answers in a system that defined three. Counted across `src`:
 * `rounded-lg` 55 times, `rounded-md` 10, `rounded-2xl` 5, `rounded-xl` 4 — Tailwind's own scale,
 * sitting near but never on the theme's radii — against 77 uses of the real token spelled the long
 * way. Nobody chose that; it is what happens when a token exists and reaching past it is one
 * character shorter.
 *
 * So the tokens are `control`, `card`, `panel`, `pill`, and this fails the build on anything else.
 * A rule is only a rule if something checks it — `theme.test.ts` next door exists for the same
 * reason, guarding the light/dark pairs against drift.
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
  /^rounded-(control|card|panel|pill|full|none)$/,
  /^rounded-[tblrse]{1,2}-(control|card|panel|pill|full|none)$/,
  /^rounded-[tblrse]{1,2}?-?\[var\(--radius-(control|card|panel|pill)\)\]$/,
  /^rounded-\[\d+%\]$/,
];

function tsxFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return tsxFiles(path);
    return path.endsWith(".tsx") ? [path] : [];
  });
}

describe("border radius", () => {
  it("is always one of the four tokens", () => {
    const offenders: string[] = [];
    for (const file of tsxFiles("src")) {
      const source = readFileSync(file, "utf8");
      for (const [line, text] of source.split("\n").entries()) {
        // Only what is being applied, not what a comment is quoting: several primitives document
        // the hand-rolled strings they replaced, and those quotations are the evidence for why the
        // primitive exists. A test that forced them to be edited would erase its own reason.
        if (/^\s*(\*|\/\/)/.test(text)) continue;
        for (const match of text.matchAll(/\brounded-[\w[\]()%-]*/g)) {
          const found = match[0];
          if (!ALLOWED.some((shape) => shape.test(found))) {
            offenders.push(`${file}:${line + 1} ${found}`);
          }
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
