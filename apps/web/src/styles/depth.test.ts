import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { callSites } from "./call-sites";

/**
 * How far off the page a thing sits, and how far in front of everything else.
 *
 * Two scales, one test, because they are the same decision asked twice: elevation is what a
 * surface looks like, stacking is what it covers. A panel drawn as floating and stacked as flat is
 * a panel that looks wrong in exactly the moment it matters, and neither half was written down.
 *
 * ## Elevation
 *
 * `LivePip` — the minimised meeting, the one panel in this app that floats over everything — was
 * written as `shadow-panel`. There is no `--shadow-panel` and never was; the token block has three
 * shadows and they are `sm`, `card` and `pop`. Tailwind emits a utility only for a variable it can
 * see, so `.shadow-panel` was absent from the built stylesheet entirely and the panel floated over
 * the page with a one-pixel border and nothing under it.
 *
 * That is the second time this repository has shipped a class naming nothing — `className="ghost"`
 * in `Library.tsx` was the first — and both were invisible for the same reason: a class that does
 * not exist does not fail, it just does not apply.
 *
 * `shadow-lg` is the other half of the pattern and the one `radius.test.ts` already knows by heart:
 * Tailwind's own scale sitting near this one and landing on none of it.
 *
 * ## Stacking
 *
 * Four numbers across fifteen files, and `z-50` was a bucket holding six things with genuinely
 * different claims on being in front: a sheet, a menu bar, a language menu, a shortcuts dialog, the
 * command palette, and the minimised meeting. Six things at one level means the winner is whichever
 * React mounted last, which is not a decision anybody made and changes when a file is reordered.
 *
 * The scale names what each layer is *for*, so a new panel has to answer "which of these am I"
 * rather than pick a number that looks free:
 *
 * - `raised` — inside one component: a dropdown over its own list, a scrubber over its track. Local
 *   to a stacking context and never in the global argument.
 * - `docked` — furniture pinned to the viewport that the app is allowed to cover. The readout.
 * - `float` — a window the user put there and expects to keep seeing. The minimised meeting.
 * - `scrim` — what dims the app.
 * - `overlay` — what the scrim is under: sheets, menus, dialogs.
 * - `command` — the palette, which opens from inside any of the above and must be on top of it.
 *
 * `-z-10` stays allowed: a negative index paints *behind* a sibling inside an `isolate`, which is
 * the sidebar's highlight sitting under its own label. That is not this scale's question.
 */
/**
 * Anchored so `transition-shadow` is not read as a shadow.
 *
 * `\b` finds a boundary after the hyphen too, so the first version reported three call sites that
 * were animating a shadow rather than setting one — `transition-shadow` and
 * `transition-[transform,box-shadow,…]`. A guard that cries about correct code gets its exceptions
 * widened until it stops guarding anything.
 */
const SHADOW = /(?<![-\w])shadow(-[\w[\]()%,_.-]*)?/g;

const SHADOW_ALLOWED = [
  // The three tiers, spelled out. Everything in `src` writes the long form; the short form is left
  // allowed rather than banned, since both resolve to the same variable.
  /^shadow-\[var\(--shadow-(sm|card|pop)\)\]$/,
  /^shadow-(sm|card|pop|none)$/,
  // Not elevation. `shadow-` is two vocabularies sharing a prefix: a drop shadow, and a ring drawn
  // with one. The recording glow and the inset accent rule are the second.
  /^shadow-\[var\(--glow-rec\)\]$/,
  /^shadow-\[inset_/,
  /^shadow-\[0_0_0_/,
];

const LAYER = /(?<![-\w])-?z-\[?[\w()-]*\]?/g;

const LAYER_ALLOWED = [
  // Spelled through `var()`, not as `z-raised`.
  //
  // Tailwind v4 generates a utility from a theme variable only for the namespaces it knows, and
  // there is no `--z-*` namespace — `z-raised` would emit nothing, which is the `shadow-panel`
  // mistake wearing a different hat. The arbitrary-value form always resolves, and `depth.test.ts`
  // has a companion check that reads the built stylesheet rather than trusting either of us.
  /^z-\[var\(--z-(raised|docked|float|scrim|overlay|command)\)\]$/,
  /^-z-10$/,
  // `z-auto` opts out rather than picking, which is the honest answer for an element that should
  // take its parent's place in the order.
  /^z-auto$/,
];

describe("elevation", () => {
  it("is always one of the three tiers", () => {
    const offenders: string[] = [];
    for (const { file, line, text } of callSites("src")) {
      for (const match of text.matchAll(SHADOW)) {
        const found = match[0];
        if (!SHADOW_ALLOWED.some((shape) => shape.test(found))) {
          offenders.push(`${file}:${line} ${found}`);
        }
      }
    }
    expect(
      offenders,
      "a shadow that is not one of `sm`, `card`, `pop`. A name the theme does not define emits no " +
        "CSS at all — `shadow-panel` left the minimised meeting with no elevation for a release.",
    ).toEqual([]);
  });
});

describe("stacking", () => {
  it("is always a named layer", () => {
    const offenders: string[] = [];
    for (const { file, line, text } of callSites("src")) {
      for (const match of text.matchAll(LAYER)) {
        const found = match[0];
        if (!LAYER_ALLOWED.some((shape) => shape.test(found))) {
          offenders.push(`${file}:${line} ${found}`);
        }
      }
    }
    expect(
      offenders,
      "a bare stacking number. Which of two floating things is in front is a decision; a number " +
        "that looks free is not one. The layers are raised · docked · float · scrim · overlay · command.",
    ).toEqual([]);
  });
});

/**
 * And the tokens reach the stylesheet.
 *
 * Every check above reads source. The bug it was written for could not be found by reading source:
 * `shadow-panel` is a perfectly ordinary-looking class, and what was wrong with it lived in the
 * *output* — Tailwind emits a utility only for a variable in a namespace it recognises, so a name
 * outside one produces nothing and says nothing.
 *
 * There is no `--z-*` namespace either, which is why the layers are spelled `z-[var(--z-float)]`
 * and not `z-float`. That is a claim about a build tool's behaviour, and a claim about a build tool
 * belongs in a test that runs the build tool.
 *
 * Skipped when there is no `dist`, so `pnpm test` on a fresh checkout is not a failure about
 * something nobody asked to be built. CI builds before it tests.
 */
/**
 * How long a transition takes, named by the job rather than by the number.
 *
 * Four values, twenty-one call sites, and — as with the spacing grid — they were already coherent.
 * 150ms for a colour under the pointer, 200ms for a press, 300ms for something moving, 500ms for a
 * fade nobody is meant to notice. Nothing said so, which is exactly how a fifth arrives: somebody
 * types the first number that feels right and it is near four others and on none of them.
 *
 * Not merged with `lib/motion.ts`. That names durations for entrances and springs, which is a
 * different question — 180ms for a thing arriving is slower than 150ms for a hover, and should be.
 * Two vocabularies are a problem when they answer the same question differently; these do not.
 */
const DURATION = /(?<![-\w])duration-\[?[\w()-]*\]?/g;

describe("transition duration", () => {
  it("is always one of the four named jobs", () => {
    const offenders: string[] = [];
    for (const { file, line, text } of callSites("src")) {
      for (const match of text.matchAll(DURATION)) {
        const found = match[0];
        if (!/^duration-\[var\(--motion-(hover|press|shift|fade)\)\]$/.test(found)) {
          offenders.push(`${file}:${line} ${found}`);
        }
      }
    }
    expect(
      offenders,
      "a transition duration that is not one of hover · press · shift · fade.",
    ).toEqual([]);
  });
});

describe("the built stylesheet", () => {
  const dist = join("dist", "assets");
  const css = existsSync(dist)
    ? readdirSync(dist)
        .filter((name) => name.endsWith(".css"))
        .map((name) => readFileSync(join(dist, name), "utf8"))
        .join("\n")
    : null;

  it.runIf(css !== null)("carries every layer and every shadow the app asks for", () => {
    const missing = [
      ...["raised", "docked", "float", "scrim", "overlay", "command"].map((n) => `--z-${n}`),
      ...["sm", "card", "pop"].map((n) => `--shadow-${n}`),
      ...["hover", "press", "shift", "fade"].map((n) => `--motion-${n}`),
    ].filter((token) => !css!.includes(`var(${token})`));

    expect(
      missing,
      "these tokens are used in `src` and emit no CSS. That is the `shadow-panel` failure: the " +
        "class is spelled correctly, the build drops it, and the element renders without it.",
    ).toEqual([]);
  });
});
