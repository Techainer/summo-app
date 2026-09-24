import { describe, expect, it } from "vitest";

import { callSites } from "./call-sites";

/**
 * A rhythm that was already there, written down so it cannot erode.
 *
 * This one is worth stating plainly, because the plan that asked for it assumed the opposite: the
 * interface was *not* spacing things at random. Counted across `src`, 1181 spacing utilities, and
 * 93.6 % of them already sat on one coherent grid. Fifteen `gap` values with no rule looked like
 * chaos from a distance and was, up close, a rhythm with a small tail on it.
 *
 * The rhythm, now that somebody has looked:
 *
 * - **2px steps up to 12px** — inside a control, between an icon and its label, between the rows
 *   of a list. 0.5 / 1 / 1.5 / 2 / 2.5 / 3, and together they are 951 of the 1181.
 * - **4px steps up to 24px** — between controls and around a card. 4 / 5 / 6.
 * - **8px steps above** — between the parts of a page. 8 / 10 / 12 / 16 / 24.
 *
 * The tail was sixteen call sites: nine `3.5` (14px), four `7` (28px), a `py-14`, and two waveform
 * marks spelling a 2px gap as `gap-[2px]` and `gap-[2.5px]` — the same three bars in two files,
 * disagreeing by half a pixel. Fifteen moved onto the grid. None of them was a decision; they are
 * what happens when nothing is watching.
 *
 * What this test deliberately does **not** do is push the 49 uses of `5` (20px) onto some rounder
 * number. Twenty is a step on the 4px tier, it is used deliberately, and moving it would be taste
 * imposed on working call sites rather than a rule being enforced.
 *
 * The two survivors are alignments rather than rhythm, and each one is exactly the sum of what it
 * lines up under:
 *
 * - `Markdown`'s `ms-5.5` — 22px = a `size-3.5` checkbox (14) + `gap-2` (8). A nested block starts
 *   where its parent's text starts.
 * - `HelpScreen`'s `ps-15` — 60px = `px-4` (16) + a `size-8` icon disc (32) + `gap-3` (12). The
 *   answer lines up under the question, not under the icon.
 *
 * Both are correct, and rounding either to the grid would visibly misalign the thing it aligns.
 * They are listed here rather than allowed by a pattern so that a third one has to be argued for in
 * this file — which is the friction that keeps the list at two.
 */
const STEPS = ["0", "0.5", "1", "1.5", "2", "2.5", "3", "4", "5", "6", "8", "10", "12", "16", "24"];

/**
 * Everything Tailwind sizes from the spacing scale, which is more than padding and margin.
 *
 * Longest first. Alternation is leftmost-wins, so a bare `gap` ahead of `gap-x` matched the `gap`
 * of `gap-x-2` and reported `gap-x` as the offending class — a false finding that hid the real
 * ones behind it.
 */
const PREFIX =
  "gap-x|gap-y|gap|px|py|pt|pb|ps|pe|pl|pr|p|mx|my|mt|mb|ms|me|ml|mr|m|space-x|space-y";

/** A bracketed value or a plain one — and no whitespace either way, so it stops at the class. */
const SPACING = new RegExp(`\\b(?:${PREFIX})-(?:\\[[^\\]]*\\]|[\\w.]+)`, "g");

const ALLOWED = [
  new RegExp(`^(?:${PREFIX})-(?:${STEPS.map((s) => s.replace(".", "\\.")).join("|")})$`),
  // `mx-auto`, `ms-auto`, `mt-auto` — centring a box or pushing it to the far end. Not a distance
  // at all: the browser computes it from whatever room is left.
  new RegExp(`^(?:m[xytbsel]?|m[rl])-auto$`),
  // A token, which is the scale reached for by name. `--settings-indent` is the width of a label
  // column plus its gap, and exists precisely so three files stop writing `162px`.
  new RegExp(`^(?:${PREFIX})-\\[var\\(--[\\w-]+\\)\\]$`),
  // One device pixel, which is below the rhythm's resolution and is never a spacing decision:
  // `Player` uses `ms-px` to optically centre a play triangle, whose visual mass sits left of its
  // bounding box. Optical correction, not layout.
  new RegExp(`^(?:${PREFIX})-px$`),
];

/**
 * Measured against something that is not the scale, which is a different question.
 *
 * A safe-area inset is whatever the hardware says it is; a twelfth of the viewport is a proportion
 * of the window. Neither has a nearest step, so neither is this test's business — the same reason
 * `radius.test.ts` lets `rounded-full` and a percentage through.
 */
const NOT_RHYTHM = /-\[(?:[\d.]+(?:vh|vw|%)|max\(|min\(|calc\(|env\()/;

/**
 * Alignments rather than rhythm. See the header for the first two.
 *
 * `Library`'s `ml-[42px]` is the honest exception: it indents a search excerpt under the result row
 * above it, and that row's leading column is `shrink-0` — as wide as whatever `timeOfDay` renders,
 * which is "09:30" in one locale and "9:30 AM" in another. No fixed number is right for both, and
 * giving the column a fixed width would clip the longer one. It is also blind to the waveform that
 * appears at `sm:`, so the excerpt sits well left of the title on a wide window.
 *
 * Left as it was, deliberately: the real fix is to lay that row out as a grid so the excerpt can be
 * a cell in it, which is a change to `Library`, not to a spacing scale. Written down here so the
 * next person finds a known limit rather than a number nobody questioned.
 */
const ALIGNMENTS = [
  "src/components/page/Markdown.tsx ms-5.5",
  "src/screens/HelpScreen.tsx ps-15",
  "src/components/Library.tsx ml-[42px]",
];

describe("spacing", () => {
  it("is always a step on the scale, or a named alignment", () => {
    const offenders: string[] = [];
    for (const { file, line, text } of callSites("src")) {
      for (const match of text.matchAll(SPACING)) {
        const found = match[0].trim();
        if (ALLOWED.some((shape) => shape.test(found))) continue;
        if (NOT_RHYTHM.test(found)) continue;
        if (ALIGNMENTS.includes(`${file} ${found}`)) continue;
        offenders.push(`${file}:${line} ${found}`);
      }
    }
    expect(offenders).toEqual([]);
  });
});
