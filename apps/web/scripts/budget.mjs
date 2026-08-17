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
 * 225 against a measured 221. The four kB is the empty states: five screens that showed grey text
 * or a wall of zeros now draw a picture, a sentence and a button, in four languages. The stickers
 * themselves are *not* in it — the drawings and the Lottie player are both behind `import()`, and
 * the animations are static files fetched only when one is on screen. Raising this is the answer
 * the message below offers, taken on purpose and written down.
 */
const BUDGET = 225;

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
