import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * The theme has a value for every colour the vault can hold, in both schemes.
 *
 * This exists because the failure it catches is invisible to everything else. The daemon owns the
 * palette; the theme owns what each name looks like; nothing in TypeScript lists them, on purpose,
 * so that there is no second copy to drift. That leaves the two ends unchecked, and the first
 * version of this feature shipped with seven of the eight dark values missing from the bundle —
 * Tailwind v4 emits only the `@theme` variables a generated utility refers to, and these are read
 * through `var()` from an inline style. Every dot but one painted transparent.
 *
 * The screenshot audit did not catch it and could not: it scores text against its background, and
 * a dot has no text. A picture would have, if anybody had looked at the right one.
 */

const read = (relative: string) =>
  readFileSync(fileURLToPath(new URL(relative, import.meta.url)), "utf8");

/** The palette names, taken from the Rust that defines them rather than restated here. */
function palette(): string[] {
  const source = read("../../../../crates/summo-vault/src/colour.rs");
  const names = [...source.matchAll(/Swatch::new\("([a-z]+)"/g)].map((m) => m[1]!);
  // A regex that silently matched nothing would make every assertion below vacuously true.
  expect(names.length, "no swatches found — has colour.rs moved or changed shape?").toBeGreaterThan(
    0,
  );
  return names;
}

/** Every `--color-swatch-*` declared inside the rule matching `blockPattern`. */
function declared(css: string, blockPattern: RegExp): Set<string> {
  const block = blockPattern.exec(css)?.[0] ?? "";
  return new Set([...block.matchAll(/--color-swatch-([a-z]+):/g)].map((m) => m[1]!));
}

describe("swatch tokens", () => {
  const css = read("./theme.css");
  const names = palette();

  it("gives every palette colour a value in the default scheme", () => {
    expect([...declared(css, /:root\s*\{[^}]*--color-swatch-[^}]*\}/)].sort()).toEqual(
      [...names].sort(),
    );
  });

  /**
   * Both schemes, because that is the reason these are names rather than hex in the first place: a
   * colour chosen on white is not a colour on `#0b0c0e`. A name with only one value would be
   * legible in one scheme and invisible in the other, which is the bug the design avoids.
   */
  it("gives every palette colour a value in light mode too", () => {
    expect(
      [...declared(css, /@media \(prefers-color-scheme: light\)\s*\{[\s\S]*?\n\s*\}\n\}/)].sort(),
    ).toEqual([...names].sort());
  });

  /**
   * `@theme` is where Tailwind reads design tokens, and it drops the ones no utility mentions.
   * These are never a utility, so putting them there is the mistake that already happened once.
   */
  it("declares them outside the block Tailwind tree-shakes", () => {
    const theme = /@theme\s*\{[\s\S]*?\n\}/.exec(css)?.[0] ?? "";
    expect(theme, "@theme not found — this test is checking nothing").not.toBe("");
    expect(theme).not.toContain("--color-swatch-");
  });
});

/**
 * Every colour a component asks for is a colour the theme defines.
 *
 * The failure this catches is silent in a way that is worth spelling out. Tailwind v4 generates a
 * utility only when it can resolve the token behind it, and generating nothing is not an error —
 * `class="text-danger"` with no `--color-danger` produces no rule, no warning, and no visible sign
 * except that the element is the colour it would have been anyway. `text-danger` was used in
 * seventeen places for over a year: the failed import, the calendar that would not fetch, the model
 * that would not download, every `role="alert"` in onboarding. All of them rendered in ordinary
 * body colour, and every check in this repository passed. The screenshot audit could not see it
 * either — grey text on the page background is a perfectly legible contrast pair.
 *
 * So the direction matters: the other tests here check that what the theme declares survives into
 * the bundle, and this one checks that what the interface *asks for* was ever declared.
 */
describe("colour utilities", () => {
  const css = read("./theme.css");

  /** Every `--color-*` the theme defines, anywhere in the file. */
  const defined = new Set([...css.matchAll(/--color-([a-z0-9-]+):/g)].map((m) => m[1]!));

  /**
   * The prefixes that are colours often enough to be worth checking, and the values they take that
   * are *not* colours.
   *
   * Tailwind overloads these: `text-sm` is a size, `bg-cover` is a fit, `border-b` is an edge. The
   * list is static, it is Tailwind's rather than ours, and a false positive costs one line here —
   * which is the right trade against a whole class of silently-dead styling.
   */
  const NOT_A_COLOUR = new Set([
    // text-
    ...["xs", "sm", "base", "lg", "xl", "2xl", "3xl", "4xl", "5xl", "6xl", "7xl", "8xl", "9xl"],
    ...["left", "center", "right", "justify", "start", "end", "balance", "pretty"],
    ...["wrap", "nowrap", "ellipsis", "clip"],
    // bg-
    ...["fixed", "local", "scroll", "cover", "contain", "none", "repeat", "no-repeat"],
    ...["top", "bottom", "auto", "clip-text", "clip-border", "clip-padding", "clip-content"],
    // border-
    ...["0", "2", "4", "8", "x", "y", "s", "e", "t", "r", "b", "l"],
    ...["solid", "dashed", "dotted", "double", "hidden", "collapse", "separate"],
    // ring- and outline-
    ...["1", "3", "inset", "offset", "dashed-2"],
    // Tailwind's own palette, used directly in a few places where a literal is honest.
    ...["white", "black", "transparent", "current", "inherit"],
  ]);

  /** Every `.tsx` and `.ts` under `src`. */
  function sources(dir: string): string[] {
    const here = fileURLToPath(new URL(dir, import.meta.url));
    const out: string[] = [];
    for (const entry of readdirSync(here, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) out.push(...sources(`${path}/`));
      else if (/\.tsx?$/.test(entry.name) && !entry.name.endsWith(".test.ts")) out.push(path);
    }
    return out;
  }

  it("has a value behind every text-, bg-, border-, ring- and outline- colour it names", () => {
    const files = sources("../");
    expect(files.length, "no sources found — this test is checking nothing").toBeGreaterThan(20);

    /** Whether Tailwind can resolve `<prefix>-<name>` to something real. */
    const resolves = (name: string): boolean => {
      if (defined.has(name) || NOT_A_COLOUR.has(name)) return true;
      // `text-meta` is a size, not a colour, and `rounded-card` a radius. Same shape, other token.
      if (new RegExp(`--(?:text|radius|font|shadow)-${name}:`).test(css)) return true;
      // `border-l-accent`, `border-b-0`, `ring-offset-bg`: an edge or an offset, then the value.
      const [head, ...rest] = name.split("-");
      if (rest.length > 0 && /^(?:x|y|s|e|t|r|b|l|offset)$/.test(head!)) {
        return resolves(rest.join("-"));
      }
      return false;
    };

    const missing = new Map<string, string>();
    for (const file of files) {
      // Comments first: this file's own prose names utilities in backticks, and a doc comment
      // explaining why `bg-soft` was rejected must not read as a use of it.
      //
      // Arbitrary values are Tailwind passing CSS straight through, and the CSS inside them says
      // things like `var(--color-bg-elevated)` and `transition-[background,border-color]` that look
      // exactly like a utility and are not one. Dropped before scanning rather than pattern-matched
      // around, which is what a lookbehind alone could not do.
      const source = read(file)
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/\/\/[^\n]*/g, "")
        .replace(/\[[^\]]*\]/g, "");
      // The name only, without the opacity suffix: `text-accent/60` is `accent`. The lookbehind is
      // what stops `--avatar-text-c` and `--color-bg-elevated` reading as utilities.
      for (const [, name] of source.matchAll(
        /(?<![\w-])(?:text|bg|border|ring|outline)-([a-z][a-z0-9]*(?:-[a-z0-9]+)*)(?:\/\d+)?/g,
      )) {
        if (resolves(name!)) continue;
        if (!missing.has(name!)) missing.set(name!, file);
      }
    }

    expect(
      [...missing].map(([name, file]) => `${name} (first seen in ${file})`),
      "these utilities generate no CSS at all",
    ).toEqual([]);
  });
});
