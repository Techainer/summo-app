import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import vi from "./vi.json";
import viMore from "./vi.more.json";

/**
 * The eager half of the catalogue holds every string the eager half of the app can show.
 *
 * The catalogue is two files per language — `vi.json` before the first pixel, `vi.more.json` when a
 * lazy screen opens — and the division is the difference between a 200 kB first load and a 193 kB
 * one. It is also the kind of division that rots silently: somebody imports a settings string into
 * the shell, everything works on their machine because the lazy half is already cached, and a user
 * on a cold load reads `settings.mt_where` where a label should be.
 *
 * So the split is not maintained by hand. This walks the *statically* imported module graph from
 * `main.tsx` — dynamic `import()` is where laziness begins, so it is deliberately not followed —
 * and asserts that every namespace those files mention is in the eager file. Add an eager import of
 * a lazy screen's copy and this fails, naming the namespace and the file.
 *
 * The check is on namespaces rather than individual keys because a namespace is the unit the files
 * are divided by, and because keys are built from prefixes in a dozen places
 * (`t(\`setup.step_${n}\`)`) that no static scan can enumerate.
 */

const HERE = dirname(fileURLToPath(import.meta.url));
const SRC = resolve(HERE, "..");

/** Resolve a relative specifier the way the bundler does, or `null` for a package. */
function fileFor(from: string, specifier: string): string | null {
  if (!specifier.startsWith(".")) return null;
  const base = resolve(dirname(from), specifier);
  for (const candidate of [
    base,
    `${base}.ts`,
    `${base}.tsx`,
    join(base, "index.ts"),
    join(base, "index.tsx"),
  ]) {
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
  }
  return null;
}

/** Every module reachable from `entry` without crossing a dynamic import. */
function eagerGraph(entry: string): string[] {
  const seen = new Set<string>();
  const walk = (file: string) => {
    if (seen.has(file)) return;
    seen.add(file);
    // `import(` becomes something the `from "…"` scan cannot see, which is the whole point: a
    // dynamic import is a chunk boundary and its contents are not in the first load.
    const source = readFileSync(file, "utf8").replace(/\bimport\s*\(/g, "DYNAMIC(");
    for (const [, specifier] of source.matchAll(/from\s+"([^"]+)"/g)) {
      const next = fileFor(file, specifier!);
      if (next) walk(next);
    }
  };
  walk(entry);
  return [...seen];
}

describe("the catalogue split", () => {
  const core = new Set(Object.keys(vi));
  const later = new Set(Object.keys(viMore));

  it("has no namespace in both halves, which would make one of them dead weight", () => {
    expect([...core].filter((key) => later.has(key))).toEqual([]);
  });

  it("keeps every string the app can show before a lazy screen opens", () => {
    const files = eagerGraph(join(SRC, "main.tsx"));
    expect(files.length, "the graph walk found nothing — has main.tsx moved?").toBeGreaterThan(20);

    const known = new Set([...core, ...later]);
    const wrongHalf = new Map<string, string>();
    for (const file of files) {
      if (file.endsWith(".test.ts") || file.endsWith(".test.tsx")) continue;
      const source = readFileSync(file, "utf8")
        // This file's own prose names namespaces; a doc comment must not read as a use.
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/\/\/[^\n]*/g, "");
      for (const [, namespace] of source.matchAll(/["'`]([a-z_]+)\.[a-z_0-9]+/g)) {
        if (!known.has(namespace!) || core.has(namespace!)) continue;
        if (!wrongHalf.has(namespace!)) wrongHalf.set(namespace!, file.slice(SRC.length + 1));
      }
    }

    expect(
      [...wrongHalf].map(([namespace, file]) => `${namespace} (used eagerly by ${file})`),
      "these namespaces are in *.more.json but the first load can reach them — move them across",
    ).toEqual([]);
  });
});
