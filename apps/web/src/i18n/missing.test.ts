import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import vi from "./vi.json";

/**
 * Every key the source asks for is in the catalogue.
 *
 * The mirror of `unused.test.ts`, and the more embarrassing direction. `t()` falls back to the key
 * it was given, so a key that does not exist is not an error, a warning, or a blank — it is the
 * literal text `setup.local` rendered in a badge on the welcome screen, which is where it sat for
 * two releases after the sentence it named was deleted and the badge was not.
 *
 * Only literal calls are checked. `t(\`setup.step_${check.step}\`)` builds its key at runtime and
 * this file cannot know which suffixes exist — that half is what `unused.test.ts` covers from the
 * other side.
 */

const HERE = new URL(".", import.meta.url).pathname;
const WEB = join(HERE, "..");

function keysOf(node: unknown, prefix = ""): string[] {
  if (typeof node !== "object" || node === null) return [prefix];
  return Object.entries(node).flatMap(([key, value]) =>
    keysOf(value, prefix ? `${prefix}.${key}` : key),
  );
}

function sources(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...sources(path));
    else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(path);
  }
  return out;
}

/** `t("meeting.stop")` and `n("import.sentences", 3)`, in either quote. */
const CALLS = /\b[tn]\(\s*["']([a-z][a-z0-9_]*(?:\.[a-z0-9_]+)+)["']/g;

/** A plural is stored as `key_one` / `key_other`; the code names it without the suffix. */
const FORMS = ["", "_zero", "_one", "_two", "_few", "_many", "_other"];

describe("keys the source asks for", () => {
  const files = sources(WEB);
  const asked = new Map<string, string>();
  for (const file of files) {
    // Comments first. Half the explanations in this codebase quote a `t("…")` call to say what a
    // helper does, and a documentation example is not a key anybody renders.
    const source = readFileSync(file, "utf8")
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .replace(/^\s*\/\/.*$/gm, "");
    for (const [, key] of source.matchAll(CALLS)) {
      if (key) asked.set(key, file.slice(WEB.length + 1));
    }
  }

  it("all exist in the catalogue", () => {
    const have = new Set(keysOf(vi));
    const missing = [...asked]
      .filter(([key]) => !FORMS.some((form) => have.has(`${key}${form}`)))
      .map(([key, file]) => `${key} (${file})`);

    expect(missing, "these render as their own key on screen").toEqual([]);
  });

  it("is being read at all", () => {
    expect(files.length).toBeGreaterThan(50);
    expect(asked.size).toBeGreaterThan(200);
  });
});
