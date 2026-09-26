/**
 * Every screen, photographed.
 *
 * The other suites assert behaviour: a route renders, a drag lands, a file changes. None of them
 * can tell whether the result is *legible* — whether a heading collides with a chip at 390 px,
 * whether dark mode leaves grey text on a grey card, whether a screen with no data looks broken
 * rather than empty. That needs a human looking at a picture, and this produces the pictures.
 *
 * It also fails loudly on console errors and on obvious layout faults it *can* measure: horizontal
 * overflow, and text whose contrast against its own background is below the WCAG AA ratio. Those
 * two catch most of what a screenshot review would otherwise have to notice by eye.
 *
 * It boots a daemon of its own, like every other suite here, so it runs in CI rather than only on
 * the machine of whoever remembers. That is the point of the change: contrast and overflow were the
 * two things nothing enforced, checked by hand, on a good day.
 *
 *   node e2e/shots.mjs                                  # its own vault
 *   node e2e/shots.mjs http://127.0.0.1:7788 7788 <tok> # a daemon you are already debugging
 *
 * `SUMMO_LOCALE` picks the language the screens are photographed in; the four-language pass runs
 * inside this file either way.
 */
import { legible } from "./legible.mjs";
import { chromium } from "playwright";
import { mkdirSync, readFileSync } from "node:fs";

import { daemon } from "./daemon.mjs";
import { SCREENS, everyRouteIsCovered } from "./screens.mjs";

/**
 * The namespaces the catalogue actually has, both halves.
 *
 * What makes a dotted word a *key* rather than text. Without this the check flagged `llama.cpp`
 * and `techainer.com` — a runtime and a domain, both written on purpose — and satisfying it would
 * have meant changing a correct screen to please a regex.
 */
const NAMESPACES = ["vi.json", "vi.more.json"].flatMap((file) =>
  Object.keys(JSON.parse(readFileSync(new URL(`../src/i18n/${file}`, import.meta.url), "utf8"))),
);

const engine = await daemon(process.argv, { name: "shots" });
const appUrl = engine.url;
const token = engine.token;
const locale = process.env.SUMMO_LOCALE ?? "vi-VN";

const OUT = "/tmp/shots";
mkdirSync(OUT, { recursive: true });

// The list, and the rule that a route cannot be quietly unphotographed — see `screens.mjs`.
everyRouteIsCovered("shots.mjs");

const VIEWPORTS = [
  ["wide", { width: 1280, height: 860 }],
  ["narrow", { width: 390, height: 844 }],
];

/**
 * Widths that are measured but not photographed.
 *
 * Two widths was two answers to a question with a range of them, and a user reported layouts
 * breaking in places neither of them covers. The gaps are the ordinary ones: 320 is the narrowest
 * phone still in use, 768 is a tablet held upright and the width just under every `sm:` and `md:`
 * branch in this app, and 1024 is a laptop beside another window — the width a person running a
 * call on one half of the screen actually has.
 *
 * Measured rather than photographed, and in one theme. The checks are what find the bugs; the PNGs
 * are for a person to look at afterwards, and nobody is going to look through a hundred and ninety
 * of them. This keeps the suite a few minutes rather than a quarter of an hour, which is the
 * difference between a check that runs on every push and one somebody turns off.
 */
const WIDTHS = [
  ["320", { width: 320, height: 720 }],
  ["768", { width: 768, height: 1024 }],
  ["1024", { width: 1024, height: 768 }],
];

const problems = [];
const browser = await chromium.launch();

for (const scheme of ["light", "dark"]) {
  for (const [width, viewport] of VIEWPORTS) {
    const context = await browser.newContext({ locale, viewport, colorScheme: scheme });
    // The quick tour is a first-run overlay. It is correct that it appears, and it covers a
    // quarter of the screen — so every picture of every screen would be a picture of the tour.
    // Marked as seen, the way it is for anybody who has used the app once.
    await context.addInitScript(() => window.localStorage.setItem("summo.tour", "done"));
    const page = await context.newPage();
    page.on("console", (m) => {
      if (m.type() === "error") problems.push(`console ${scheme}/${width}: ${m.text()}`);
    });
    page.on("pageerror", (e) => problems.push(`pageerror ${scheme}/${width}: ${e.message}`));

    for (const [name, route] of SCREENS) {
      if (route === null) continue;
      await page.goto(`${appUrl}/?token=${token}#${route}`, { waitUntil: "networkidle" });
      // The router paints after hydration; the shell header is the first thing that exists.
      await page.locator("header, main").first().waitFor({ timeout: 10000 });
      // Motion runs an entrance on most screens. Let it finish so the picture is the resting state.
      await page.waitForTimeout(700);
      await page.screenshot({ path: `${OUT}/${scheme}-${width}-${name}.png`, fullPage: false });

      await legible(page, `${scheme}/${width}/${name}`, problems);

      // A key name where a sentence should be.
      //
      // `t()` falls back to the key it was given, so an untranslated string is not an error or a
      // blank — it is the literal text `settings.mt_where` sitting in a form. Two tests already
      // guard the catalogue's *contents*; this guards its *delivery*, which became a separate thing
      // the moment the catalogue was split into an eager half and a lazy one. A namespace filed in
      // the wrong half renders exactly this, on exactly these screens, and only on a cold load.
      const keyNames = await page.evaluate((namespaces) => {
        const shape = /^[a-z][a-z_]*\.[a-z_][a-z_0-9]*$/;
        const known = new Set(namespaces);
        const found = new Set();
        for (const element of document.querySelectorAll("body *")) {
          for (const node of element.childNodes) {
            if (node.nodeType !== Node.TEXT_NODE) continue;
            const text = (node.textContent ?? "").trim();
            // The shape alone is not enough. `llama.cpp` is a runtime's name and `techainer.com`
            // is a domain, and both are text somebody wrote on purpose — flagging them would have
            // had the fix be a worse screen. A key is a key only if the catalogue has that
            // namespace, which is what `t()` would have been looking in when it gave up.
            if (shape.test(text) && known.has(text.split(".")[0])) found.add(text);
          }
        }
        return [...found];
      }, NAMESPACES);
      for (const key of keyNames) {
        problems.push(`${scheme}/${width}/${name}: untranslated key on screen — ${key}`);
      }
    }

    await context.close();
  }
}

// The widths nobody photographs. See `WIDTHS`: same checks, one theme, no PNGs.
for (const [width, viewport] of WIDTHS) {
  const context = await browser.newContext({ locale, viewport, colorScheme: "dark" });
  await context.addInitScript(() => window.localStorage.setItem("summo.tour", "done"));
  const page = await context.newPage();
  page.on("console", (m) => {
    if (m.type() === "error") problems.push(`console ${width}px: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror ${width}px: ${e.message}`));

  for (const [name, route] of SCREENS) {
    if (route === null) continue;
    await page.goto(`${appUrl}/?token=${token}#${route}`, { waitUntil: "networkidle" });
    await page.locator("header, main").first().waitFor({ timeout: 10000 });
    await page.waitForTimeout(500);
    await legible(page, `${width}px/${name}`, problems);
  }

  await context.close();
}

await browser.close();
await engine.stop();

if (problems.length) {
  console.error(`\n${problems.length} problem(s):`);
  // Repeats are the same token used on every screen; showing each once is what is actionable.
  for (const p of [...new Set(problems)]) console.error(`  ${p}`);
  process.exit(1);
}
console.log(`shots in ${OUT}, no console errors, no overflow, contrast AA everywhere`);
