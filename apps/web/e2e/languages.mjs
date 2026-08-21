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
 * - **A key the catalogue does not have.** The translator says so — `[i18n] missing key: …` —
 *   and it says it as a `console.warn`, which every suite here ignored while failing on
 *   `console.error`. So a key built at runtime, `t(`setup.step_${step}`)`, could be absent from
 *   every catalogue and the only sign would be its own name rendered on screen in a language
 *   nobody reading the tests speaks. Unit tests cannot reach those keys: they exist only once
 *   something composes them. This is the pass that renders them, so this is where it is caught.
 */
import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";

const LANGUAGES = ["vi", "en", "ja", "zh"];

/**
 * Every screen with a route of its own, visited in every language.
 *
 * It was six, which was enough for the overflow and heading checks — those fail on whichever
 * screen is worst and one bad screen is enough to know. It is not enough for the missing-key
 * check: a key is only composed when the screen that composes it renders, so a screen nothing
 * visits is a screen whose keys nothing checks.
 *
 * The two routes that take a parameter are left out. They need a vault entry to point at, which
 * the meeting and page suites already cover; the rest of this file is about language, not content.
 */
const SCREENS = [
  "/",
  "/record",
  "/library",
  "/notes",
  "/agenda",
  "/chat",
  "/agents",
  "/tasks",
  "/people",
  "/analytics",
  "/models",
  "/settings",
  "/help",
];

/** The screens compared against Vietnamese: the ones whose first heading is a translated title. */
const HEADINGS = ["/", "/library", "/tasks", "/chat", "/analytics", "/settings"];

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

    // Reported once per key per page by the translator itself, and deduped again here so one
    // missing key on a screen visited in four languages is four lines rather than forty.
    const missing = new Set();
    page.on("console", (message) => {
      const said = message.text();
      if (said.includes("[i18n] missing key")) missing.add(said.split(":").pop().trim());
      if (message.type() === "error") fail(`${locale}: console — ${said}`);
    });
    page.on("pageerror", (e) => fail(`${locale}: pageerror — ${e.message}`));

    // Set before the first paint: the app reads the saved choice on load, and switching afterwards
    // would test a re-render rather than a cold start in that language.
    await page.addInitScript((value) => {
      window.localStorage.setItem("summo.locale", value);
    }, locale);

    headings[locale] = {};

    for (const screen of SCREENS) {
      await page.goto(`${daemon.url}#${screen}`);
      await page.waitForTimeout(700);

      const h1 = await page
        .locator("h1, h2")
        .first()
        .innerText()
        .catch(() => "");
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

    for (const key of missing) fail(`${locale}: no translation for ${key} — it rendered its key`);

    console.log(`${locale}: ${HEADINGS.map((s) => headings[locale][s]).join(" | ")}`);
    await context.close();
  }

  // A screen whose heading is identical to the Vietnamese one is a screen that did not translate.
  // Compared per screen rather than per language so the message names which one.
  for (const locale of LANGUAGES.filter((l) => l !== "vi")) {
    for (const screen of HEADINGS) {
      const source = headings.vi[screen];
      if (source && source === headings[locale][screen]) {
        fail(`${locale} ${screen}: heading is still the Vietnamese one — “${source}”`);
      }
    }
  }

  /**
   * A first run, in a machine's own language.
   *
   * Everything above writes `summo.locale` before loading the page, which is the right way to test
   * that a language *renders* — and it means every case above is one where the user has already
   * chosen. Nothing covered what happens when they have not, which is the state every single
   * install passes through exactly once and the one nobody developing this ever sees again.
   *
   * So: a fresh context per system locale, no storage, and the only signal is `navigator.languages`
   * — which in the packaged desktop shell is what the operating system is set to. `ko` is in the
   * list because Summo ships no Korean: a language with no catalogue has to land on English rather
   * than on key names or a blank screen.
   */
  const FIRST_RUN = [
    { system: "vi-VN", expect: "vi" },
    { system: "ja-JP", expect: "ja" },
    { system: "zh-CN", expect: "zh" },
    { system: "en-GB", expect: "en" },
    { system: "ko-KR", expect: "en" },
  ];

  for (const { system, expect } of FIRST_RUN) {
    const context = await browser.newContext({
      locale: system,
      viewport: { width: 1280, height: 860 },
    });
    const page = await context.newPage();
    await page.goto(daemon.url, { waitUntil: "domcontentloaded" });
    await page.locator("header").waitFor({ timeout: 20000 });
    // `<html lang>` is set from the active locale, and is the one place the answer is a bare tag
    // rather than a sentence somebody has to recognise in a language they may not read.
    const got = await page.evaluate(() => document.documentElement.lang);
    const saved = await page.evaluate(() => window.localStorage.getItem("summo.locale"));
    if (got !== expect) {
      fail(`first run on a ${system} machine opened in ${got}, not ${expect}`);
    }
    // Detection must not *write* a choice. If it did, a later change of system language — or the
    // download page's handoff link — would be silently outranked by a preference nobody set.
    if (saved !== null) {
      fail(`first run on a ${system} machine saved ${saved} without being asked`);
    }
    console.log(`first run ${system} → ${got}`);
    await context.close();
  }

  /**
   * The language the download page hands over, via `summo://lang/<code>`.
   *
   * The shell turns that URL into a Tauri event and `bridgeShellEvents` turns it into the DOM event
   * dispatched below — outside a shell the bridge is a no-op, so the browser stands in for it here.
   * That seam is the right place to cut: the shell's half is unit-tested in `main.rs`, where the
   * hostile URLs live, and everything downstream of the event is what this exercises.
   *
   * Three rules, and the second two matter more than the first:
   *
   * - it applies on a first run, which is the case it exists for;
   * - it never overrides a language the user chose, or revisiting the download page would silently
   *   re-language an app somebody has used for a month;
   * - a code Summo does not ship is ignored rather than fallen back on, because falling back would
   *   put English on a machine whose own locale had already chosen better.
   */
  const offer = async (page, code) => {
    await page.evaluate((c) => {
      window.dispatchEvent(new CustomEvent("summo:set-locale", { detail: c }));
    }, code);
    await page.waitForTimeout(600);
    return page.evaluate(() => document.documentElement.lang);
  };

  const HANDOFF = [
    { system: "en-GB", saved: null, code: "vi", expect: "vi", why: "a first run takes the offer" },
    { system: "en-GB", saved: "ja", code: "vi", expect: "ja", why: "a chosen language wins" },
    { system: "vi-VN", saved: null, code: "ko", expect: "vi", why: "an unshipped code is ignored" },
  ];

  for (const { system, saved, code, expect, why } of HANDOFF) {
    const context = await browser.newContext({
      locale: system,
      viewport: { width: 1280, height: 860 },
    });
    const page = await context.newPage();
    if (saved) {
      await page.addInitScript((v) => window.localStorage.setItem("summo.locale", v), saved);
    }
    await page.goto(daemon.url, { waitUntil: "domcontentloaded" });
    await page.locator("header").waitFor({ timeout: 20000 });
    const got = await offer(page, code);
    if (got !== expect) {
      fail(`summo://lang/${code} on a ${system} machine (saved=${saved}): ${why} — got ${got}`);
    }
    console.log(`handoff ${system} saved=${saved} + lang/${code} → ${got}`);
    await context.close();
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
