/**
 * What the browser has to download before it can paint anything.
 *
 * A budget rather than a report, because a report is a number nobody reads until somebody
 * complains that the app takes a while to open — and by then the growth is a year of small
 * additions, none of which looked like the problem. The rich note editor is 127 kB gzipped on its
 * own; the only reason it does not count here is that it is loaded when a note is opened, and this
 * file is what keeps that true.
 *
 * Measured from `dist/index.html`: the entry script and everything the bundler asked the browser to
 * preload alongside it. That is the real first-load cost, and it excludes a lazy chunk by
 * construction rather than by a list somebody has to maintain.
 *
 * Gzipped, because that is what crosses the wire. The daemon serves the interface from localhost on
 * a desktop — but the same build is what a phone loads over a hotel connection, and that is the
 * case worth being strict about.
 */
import { gzipSync } from "node:zlib";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * Kilobytes, gzipped, for everything needed before the first screen appears.
 *
 * 220 against a measured 208. It was 300 against a measured 277, which is a budget that had stopped
 * saying anything: it left room for a screen and a half of growth nobody would have to justify.
 * Splitting the screens out of the entry chunk and fetching the animation engine on the side freed
 * 69 kB, and lowering the number in the same commit is what keeps that free rather than spent.
 *
 * Twelve kB of headroom is deliberate. A dependency bump moving a few kB should not fail a build
 * that changed nothing; a new screen imported eagerly should.
 *
 * 234 against a measured 230. The last nine kB are text: the empty states, and then the in-app
 * manual — a paragraph per question, times four languages, because `i18n/index.ts` imports all four
 * locale files and every user therefore carries three languages they cannot read.
 *
 * That is the thing to fix, and it is worth more than it costs to raise this: splitting the locales
 * so only the chosen one is in the entry chunk takes about fifteen kB *out*, and it makes every
 * future sentence cost a quarter of what it costs today. It is not done here because `t()` is
 * synchronous from the first render and making it asynchronous is a change to every screen, which
 * is not a thing to do at the end of a long day. Written down so it is a decision rather than a
 * drift.
 */
const BUDGET = 234;

const dist = join(import.meta.dirname, "..", "dist");
const html = readFileSync(join(dist, "index.html"), "utf8");

// `src` on the entry script, `href` on the preloads and the stylesheet. One pattern rather than
// three, because what is being counted is "a file the browser fetches for this document" and the
// attribute it arrived under is not part of that question.
const assets = [...html.matchAll(/(?:src|href)="\.?\/?(assets\/[^"]+)"/g)].map((m) => m[1]);
if (assets.length === 0) {
  console.error("budget: no assets found in dist/index.html — was the app built?");
  process.exit(2);
}

let total = 0;
const rows = assets
  .map((asset) => {
    const size = gzipSync(readFileSync(join(dist, asset))).length / 1024;
    total += size;
    return { asset, size };
  })
  .sort((a, b) => b.size - a.size);

for (const row of rows) console.log(`  ${row.size.toFixed(1).padStart(7)} kB  ${row.asset}`);
console.log(`  ${total.toFixed(1).padStart(7)} kB  first load, gzipped (budget ${BUDGET} kB)`);

if (total > BUDGET) {
  console.error(
    `\nbudget: first load is ${total.toFixed(1)} kB gzipped, over the ${BUDGET} kB budget.\n` +
      "Either make it smaller, or move what grew behind a dynamic import so it is fetched when it\n" +
      "is used. Raising the number is also an answer — but it should be a decision somebody makes\n" +
      "on purpose, which is the whole reason this file exists.",
  );
  process.exit(1);
}
