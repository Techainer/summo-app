import { readFileSync } from "node:fs";
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
    expect([
      ...declared(css, /@media \(prefers-color-scheme: light\)\s*\{[\s\S]*?\n\s*\}\n\}/),
    ].sort()).toEqual([...names].sort());
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
