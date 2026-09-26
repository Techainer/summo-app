import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const HERE = dirname(fileURLToPath(import.meta.url));

/**
 * Every suite in this directory is run by CI, is a helper, or is written down as manual.
 *
 * `ui-shots.mjs` was none of those for a release. It was written, it was given a `pnpm` script, and
 * nothing ever called that script — so the only suite that photographs the component library never
 * ran outside the machine it was written on. It was not idle while it was unwired: the first run
 * after CI picked it up found a destructive button below AA contrast in the dark theme, three
 * labels rendering the literal words `// i18n-exempt`, and three of four alert tones drawn with a
 * tick. None of that was hard to see. Nothing was looking.
 *
 * This is the same guard `shots.mjs` grew for routes — every path declared in `router.tsx` must be
 * photographed or written down as skipped with a reason — pointed one level up, at the suites
 * themselves. The failure mode is identical and so is the fix: a new thing cannot be quietly
 * unmeasured, it has to be argued for here.
 *
 * Deliberately reads `ci.yml` rather than trusting `package.json`. A script that exists and is
 * never invoked is exactly what went wrong; a list of scripts would have called `ui-shots` covered.
 */

/** Imported by other suites rather than run. `_shot` is a two-line tool for looking by hand. */
const HELPERS = new Set(["daemon", "legible", "llm", "mirror", "screens", "_shot"]);

/**
 * Suites that are meant to be run by a person, with the reason each one is.
 *
 * Neither asserts anything, so putting them in CI would add minutes and catch nothing.
 *
 * - `review` — photographs every page in the order somebody meets them, for a human to look
 *   through. `shots.mjs` already makes the judgements a machine can make.
 * - `site-shots` — regenerates the marketing site's images into the *other* repository. It writes
 *   outside this one, which is not something CI should do on a pull request.
 * - `subtitle-latency` — prints numbers rather than asserting them. A latency threshold on a shared
 *   runner is a test people re-run until it passes, which is worse than no test; and the run costs
 *   a 610 MB model and 75 seconds of deliberately real-time audio. Run it when touching anything
 *   between a finished utterance and a subtitle — it is what found that a live translation run is
 *   always a single line, after a commit message had claimed otherwise.
 */
const BY_HAND = {
  review: "a walkthrough for a person to look at",
  "site-shots": "writes to the site repo",
  "subtitle-latency": "measures rather than asserts; a threshold here would be re-run until green",
};

function suites() {
  return readdirSync(join(HERE))
    .filter((name) => name.endsWith(".mjs") && !name.endsWith(".test.mjs"))
    .map((name) => name.slice(0, -4));
}

/** What CI actually invokes: suites by path, plus whatever the `pnpm` scripts it names expand to. */
function runByCI() {
  const ci = readFileSync(join(HERE, "../../../.github/workflows/ci.yml"), "utf8");
  const scripts = JSON.parse(readFileSync(join(HERE, "../package.json"), "utf8")).scripts;
  const run = new Set([...ci.matchAll(/e2e\/([\w-]+)\.mjs/g)].map((m) => m[1]));
  for (const [, name] of ci.matchAll(/pnpm -C apps\/web ([\w:-]+)/g)) {
    for (const m of (scripts[name] ?? "").matchAll(/e2e\/([\w-]+)\.mjs/g)) run.add(m[1]);
  }
  return run;
}

describe("browser suites", () => {
  it("are all run by CI, or written down as not", () => {
    const run = runByCI();
    const unaccounted = suites().filter(
      (name) => !run.has(name) && !HELPERS.has(name) && !(name in BY_HAND),
    );
    expect(unaccounted).toEqual([]);
  });

  it("do not claim to run something that is gone", () => {
    const have = new Set([...suites(), ...HELPERS]);
    const missing = [...runByCI(), ...Object.keys(BY_HAND)].filter((name) => !have.has(name));
    expect(missing).toEqual([]);
  });
});
