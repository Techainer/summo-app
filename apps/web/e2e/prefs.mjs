/**
 * The two preferences that are now in the top bar: language, and light or dark.
 *
 * Both of them existed before this suite and neither was reachable without knowing where to look —
 * Cài đặt → Chung, or the command palette. That is fine for "how long before the screen locks" and
 * wrong for the two settings somebody changes *because of what is on the screen in front of them*:
 * the app opened in a language they cannot read, or the room got dark.
 *
 * Everything here is a click on a real control in a real browser, because that is the only thing
 * that catches the failure this class of change actually has: a button wired to a handler that
 * updates a state nothing renders from. The theme lived in `lib/theme.ts` for a release with two
 * fully-written blocks of CSS behind it and nothing ever setting the attribute.
 */
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "prefs" });
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 860 },
});
const page = await context.newPage();
const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") problems.push(`console: ${m.text()}`);
});

const open = async () => {
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
    waitUntil: "networkidle",
  });
  await page.locator("header").waitFor({ timeout: 10000 });
};

const scheme = () => page.evaluate(() => document.documentElement.getAttribute("data-theme"));

await open();

// ---- light or dark, from the header --------------------------------------
{
  const button = page.getByRole("button", { name: "Giao diện" });
  await button.waitFor({ timeout: 10000 });

  // Nothing set: the page is following the operating system, which is what a default has to be.
  if ((await scheme()) !== null) {
    problems.push(`a fresh app should follow the system, but data-theme is ${await scheme()}`);
  }

  const order = ["light", "dark", null];
  for (const expected of order) {
    await button.click();
    await page.waitForTimeout(120);
    const now = await scheme();
    if (now !== expected) {
      problems.push(`clicking the theme button gave ${JSON.stringify(now)}, expected ${expected}`);
      break;
    }
  }

  // Chosen once, kept afterwards — the reason this is in `localStorage` and not in React state.
  await button.click(); // light
  await open();
  if ((await scheme()) !== "light") {
    problems.push(`the theme did not survive a reload: ${await scheme()}`);
  }
  // Back to following the machine, so the language checks below start from a normal app.
  await page.getByRole("button", { name: "Giao diện" }).click();
  await page.getByRole("button", { name: "Giao diện" }).click();
  await page.waitForTimeout(120);
}

// ---- the interface language, from the header ------------------------------
{
  const trigger = page.getByRole("button", { name: "Ngôn ngữ" });
  await trigger.waitFor({ timeout: 10000 });

  const tag = (await trigger.innerText()).trim();
  if (tag !== "VI")
    problems.push(`the language button should name the current language, got "${tag}"`);

  await trigger.click();
  await page.getByRole("menuitem", { name: "English" }).click();
  await page.waitForTimeout(400);

  const lang = await page.evaluate(() => document.documentElement.lang);
  if (lang !== "en") problems.push(`choosing English left the document in ${JSON.stringify(lang)}`);

  // The document attribute is what a screen reader reads; this is what the person reads.
  const nav = await page.getByRole("navigation", { name: "Screens" }).count();
  if (nav === 0) problems.push("the interface did not re-render in English");

  await open();
  if ((await page.evaluate(() => document.documentElement.lang)) !== "en") {
    problems.push("the language did not survive a reload");
  }

  // Back to Vietnamese from the English interface, which is the return trip somebody who pressed
  // the wrong one has to make.
  await page.getByRole("button", { name: "Language" }).click();
  await page.getByRole("menuitem", { name: "Tiếng Việt" }).click();
  await page.waitForTimeout(400);
  if ((await page.evaluate(() => document.documentElement.lang)) !== "vi") {
    problems.push("could not get back to Vietnamese");
  }
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("prefs ok");
