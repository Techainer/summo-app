/**
 * The app in every language it ships in.
 *
 * The unit tests already prove the catalogues are complete and that their placeholders match. What
 * they cannot see is the part that only exists on screen: Japanese and Chinese have no spaces, so a
 * label that wrapped comfortably in Vietnamese becomes one unbreakable run of glyphs, and the first
 * thing that happens is a button growing wider than the pane it is in.
 *
 * So this walks the screens in each language and looks for two things a passing unit test would
 * never catch:
 *
 * - **Anything wider than the viewport.** A sidebar item that pushes the layout sideways is the
 *   characteristic CJK failure and is invisible to anything asserting on text.
 * - **Text that did not change language.** Every screen is compared against the same screen in
 *   Vietnamese: if a heading is byte-identical in `ja`, either the key is missing or something is
 *   hardcoded, and both look the same to a reader who cannot tell.
 */
import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";

const LANGUAGES = ["vi", "en", "ja", "zh"];

/** Nav keys, and the screens they lead to. Labels differ per language, so route by hash. */
const SCREENS = ["/", "/library", "/tasks", "/chat", "/analytics", "/settings"];

const problems = [];
const fail = (message) => problems.push(message);

const daemon = await boot(process.argv, { name: "languages" });
const browser = await chromium.launch();

/** Headings seen per language, keyed by screen, so `vi` can be compared against the rest. */
const headings = {};

try {
  for (const locale of LANGUAGES) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 860 } });
    const page = await context.newPage();

    // Set before the first paint: the app reads the saved choice on load, and switching afterwards
    // would test a re-render rather than a cold start in that language.
    await page.addInitScript((value) => {
      window.localStorage.setItem("summo.locale", value);
    }, locale);

    headings[locale] = {};

    for (const screen of SCREENS) {
      await page.goto(`${daemon.url}#${screen}`);
      await page.waitForTimeout(700);

      const h1 = await page.locator("h1, h2").first().innerText().catch(() => "");
      headings[locale][screen] = h1.trim();

      // Anything sticking out of the window. `documentElement` rather than `body`: a fixed sidebar
      // overflowing does not widen the body.
      const overflow = await page.evaluate(() => {
        const doc = document.documentElement;
        if (doc.scrollWidth <= doc.clientWidth + 1) return null;
        for (const el of document.querySelectorAll("*")) {
          const box = el.getBoundingClientRect();
          if (box.right > doc.clientWidth + 1 && box.width > 0) {
            return `${el.tagName.toLowerCase()}.${el.className}`.slice(0, 120);
          }
        }
        return "something";
      });
      if (overflow) fail(`${locale} ${screen}: pushed past the viewport — ${overflow}`);

      await page.screenshot({ path: `/tmp/shots/lang-${locale}${screen.replace(/\//g, "-")}.png` });
    }

    console.log(`${locale}: ${SCREENS.map((s) => headings[locale][s]).join(" | ")}`);
    await context.close();
  }

  // A screen whose heading is identical to the Vietnamese one is a screen that did not translate.
  // Compared per screen rather than per language so the message names which one.
  for (const locale of LANGUAGES.filter((l) => l !== "vi")) {
    for (const screen of SCREENS) {
      const source = headings.vi[screen];
      if (source && source === headings[locale][screen]) {
        fail(`${locale} ${screen}: heading is still the Vietnamese one — “${source}”`);
      }
    }
  }
} finally {
  await browser.close();
  daemon.stop();
}

if (problems.length) {
  for (const problem of problems) console.error(`FAIL ${problem}`);
  process.exit(1);
}
console.log("\nno problems");
