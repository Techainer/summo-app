import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import vi from "./vi.json";

/**
 * Every string in the catalogue has somewhere it can appear.
 *
 * The source language had 790 keys and 132 of them were unreachable — each translated into three
 * other languages, so roughly five hundred sentences nobody could ever read. Some were simply left
 * behind by a screen that changed. The dangerous ones were the near-duplicates:
 * `record.compact_while_recording`, `record.collapse_window` and `record.expand_window` are copy
 * written for the compact window, which shipped using `nav.shrink` and `nav.overlay` — two sets of
 * words for one control, and the next person to reword it would have found the wrong one.
 *
 * A key can be reached two ways, and both count here:
 *
 * - written out, `t("meeting.stop")`
 * - built from a prefix, `t(\`setup.step_${check.step}\`)` — so anything under a prefix the source
 *   interpolates is reachable, and this file cannot know which suffixes exist
 *
 * Vietnamese only. It is the source language: the others are translations of it, and a key missing
 * from them is a translation nobody has written yet rather than dead weight.
 */

const HERE = new URL(".", import.meta.url).pathname;
const WEB = join(HERE, "..");
const E2E = join(HERE, "../../e2e");

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
    else if (/\.(tsx?|mjs)$/.test(entry)) out.push(path);
  }
  return out;
}

const code = [...sources(WEB), ...sources(E2E)]
  .filter((file) => !file.endsWith("unused.test.ts"))
  .map((file) => readFileSync(file, "utf8"))
  .join("\n");

/**
 * `key_one` and `key_other` are one key as far as the source is concerned.
 *
 * `n("import.sentences", count)` picks the form at runtime — the suffix never appears in the code,
 * so without this every plural in the catalogue reads as dead.
 */
const PLURAL = /_(zero|one|two|few|many|other)$/;

/** `theme.${scheme}`, `setup.step_${check.step}` — everything under the prefix is reachable. */
const prefixes = [...code.matchAll(/[`"']([a-z_]+(?:\.[a-z_]+)*[._])\$\{/g)].map(
  (match) => match[1] ?? "",
);

describe("the Vietnamese catalogue", () => {
  it("has no strings nothing can show", () => {
    const unreachable = keysOf(vi).filter(
      (key) =>
        !code.includes(key) &&
        !code.includes(key.replace(PLURAL, "")) &&
        !prefixes.some((prefix) => key.startsWith(prefix)),
    );

    expect(unreachable, "unreachable copy — delete it or wire it up").toEqual([]);
  });

  it("is being read at all", () => {
    // The check on the check: a catalogue that failed to import, or a source sweep that found no
    // files, would make the assertion above pass by testing nothing.
    expect(keysOf(vi).length).toBeGreaterThan(400);
    expect(code.length).toBeGreaterThan(100_000);
    expect(prefixes.length).toBeGreaterThan(5);
  });
});
