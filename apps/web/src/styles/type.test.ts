import { describe, expect, it } from "vitest";

import { callSites } from "./call-sites";

/**
 * Seven steps, and a test so the eighth has to be argued for.
 *
 * The scale in `theme.css` had five steps and the components used sixty-two sizes that were not on
 * it: `text-sm` forty-eight times, `text-xl` and `text-2xl` and `text-base` and `text-xs`, plus
 * six written straight out as `text-[22px]`, `text-[13px]`, `text-[10px]`, `text-[0.6rem]`,
 * `text-[0.7rem]`. Tailwind's own scale sits near this one everywhere and lands on it nowhere —
 * 14px against 15px, 12px against 13px — so every one of those sites was a size nothing else in
 * the interface used.
 *
 * Two of them were the same words at two sizes: `Library` drew a document title at `text-2xl` and
 * the field for editing that title at `text-[22px]`, so the heading changed size when you clicked
 * into it. That is the shape of the whole problem — nobody decided it, it is just what happens when
 * reaching past a token is shorter than reaching for one.
 *
 * Allowed on purpose:
 *
 * - `text-[0.9em]` — inline code inside prose, sized *relative to whatever it sits in*. That is a
 *   different question from "how big is this text", and no absolute step can answer it.
 * - colours, alignment, weight, decoration, transform. `text-` is three vocabularies sharing a
 *   prefix, and only one of them is a scale.
 */
const SIZES = ["display", "heading", "subhead", "title", "body", "meta", "micro"];

/** `text-` also spells colour, alignment and wrapping; only the sizes are this test's business. */
const NOT_A_SIZE =
  /^text-(left|right|center|justify|start|end|balance|pretty|nowrap|wrap|clip|ellipsis|top|bottom|middle)$/;

const ALLOWED = [
  new RegExp(`^text-(${SIZES.join("|")})$`),
  new RegExp(`^text-\\[var\\(--text-(${SIZES.join("|")})\\)\\]$`),
  // Relative to the surrounding line rather than to the scale.
  /^text-\[[\d.]+em\]$/,
];

/** A size is a bare word or a bracket; a colour is a token name this file has no opinion about. */
const RAW = /^text-(xs|sm|base|lg|[2-9]?xl|\[[^\]]+\])$/;

describe("type scale", () => {
  it("is always one of the seven steps", () => {
    const offenders: string[] = [];
    for (const { file, line, text } of callSites("src")) {
      for (const match of text.matchAll(/\btext-[\w[\]().%/-]*/g)) {
        const found = match[0];
        if (NOT_A_SIZE.test(found)) continue;
        if (!RAW.test(found)) continue;
        if (ALLOWED.some((shape) => shape.test(found))) continue;
        offenders.push(`${file}:${line} ${found}`);
      }
    }
    expect(offenders).toEqual([]);
  });
});
