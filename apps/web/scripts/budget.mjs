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
import { readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";

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
 * 200 against a measured 194, and the locales are now one file each. Splitting them took 52 kB out
 * of the entry chunk and put 14 back as the one catalogue a reader actually needs — a real saving
 * of about thirty-eight, and rather more than the fifteen this comment used to estimate, because
 * four copies of the same key names compressed better together than they cost apart. It also
 * changes the slope: a new sentence of copy now costs its own bytes rather than four times them.
 *
 * 208 against a measured 200. Raised on purpose, which is the answer this file has always said is
 * available as long as somebody makes it out loud.
 *
 * Four changes crossed it in one release — the in-meeting controls, download progress with a rate,
 * the permission repairs, and measured accuracy and speed on the model cards — and the honest
 * accounting is that almost all of it is *copy*, not code. The controls are behind `lazy`, the
 * models screen is a lazy route, and the two helpers it needed moved there rather than staying in
 * `catalogue.ts`; what is left in the first load is a locale catalogue that ships whole.
 *
 * The number was grazed four times getting here — 199.5, 199.6, 199.7, 200.0 — and each time the
 * cheap fix was to shorten a sentence. That is a budget acting as a tripwire on the writing rather
 * than a brake on the bundle, which is not what it is for. Eight kB restores the headroom the
 * comment above already argues for.
 *
 * 200 against a measured 194.7. The structural fix this comment kept prescribing is done: the
 * locale catalogue is two files per language, and the second one holds the copy only a lazy screen
 * can show — the settings form, the in-app manual, the agent roster, the analytics labels. It is
 * fetched on idle beside the screen chunks and awaited by every lazy route, so nothing waits on it
 * and no screen can render a key name because of it. Worth 5.5 kB, and it changes the slope again:
 * a paragraph written for the settings screen no longer costs the first load anything at all.
 *
 * `src/i18n/split.test.ts` is what keeps it true — it walks the statically-imported graph from
 * `main.tsx` and fails if the shell can reach a string from the lazy half.
 */
const BUDGET = 200;

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

/**
 * The one language file, which the browser fetches before it paints and the document does not name.
 *
 * A dynamic import, so it is not in `index.html` and the rule above misses it — but the interface
 * cannot label a single button without it, so leaving it out would make this number describe a
 * first load nobody has. Counted at its largest, because which one is needed depends on who is
 * reading, and a budget that only holds for the cheapest reader is not a budget.
 *
 * Matched by chunk name against the catalogues in the source. That is a coupling to how Vite names
 * a chunk after its module, and it is checked rather than assumed: a rename that breaks the match
 * fails the build here instead of quietly dropping fourteen kB off the reported total.
 */
const locales = readdirSync(join(import.meta.dirname, "..", "src", "i18n"))
  .filter((file) => file.endsWith(".json"))
  .map((file) => basename(file, ".json"));

const chunks = readdirSync(join(dist, "assets"));
const localeChunks = locales
  .map((code) => chunks.find((chunk) => new RegExp(`^${code}-[A-Za-z0-9_-]+\\.js$`).test(chunk)))
  .filter((chunk) => chunk !== undefined);

if (localeChunks.length !== locales.length) {
  console.error(
    `budget: found ${localeChunks.length} locale chunks for ${locales.length} catalogues.\n` +
      "Either a language stopped being split out of the entry chunk — which is the regression this\n" +
      "check exists for — or the chunk is no longer named after its file and this script needs to\n" +
      "learn the new name.",
  );
  process.exit(2);
}

const weigh = (asset) => ({ asset, size: gzipSync(readFileSync(join(dist, asset))).length / 1024 });

const heaviestLocale = localeChunks
  .map((chunk) => weigh(`assets/${chunk}`))
  .sort((a, b) => b.size - a.size)[0];

const rows = [...assets.map(weigh), { ...heaviestLocale, note: "one language, the largest" }].sort(
  (a, b) => b.size - a.size,
);
const total = rows.reduce((sum, row) => sum + row.size, 0);

for (const row of rows) {
  const note = row.note ? `  (${row.note})` : "";
  console.log(`  ${row.size.toFixed(1).padStart(7)} kB  ${row.asset}${note}`);
}
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
