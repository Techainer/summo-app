/**
 * Light and dark, compared as pixels.
 *
 * `prefs.mjs` asserted that pressing the control set `data-theme` on <html>. It did. The attribute
 * carried no colours: `:root` is the dark palette and light lived only inside a
 * `prefers-color-scheme: light` media query, so on a machine whose system is dark, choosing Light
 * changed the scrollbars and nothing else. A user pressed it, saw no change, and said so — while
 * the suite covering that button was green.
 *
 * So this reads the painted background instead of the DOM.
 */
import { chromium } from "playwright";

import { boot } from "./daemon.mjs";

const engine = await boot({ name: "theme-shot" });
const browser = await chromium.launch();
const problems = [];

/** Average lightness of the page background, 0 (black) to 255 (white). */
const bg = (page) =>
  page.evaluate(() => {
    const colour = getComputedStyle(document.body).backgroundColor;
    const [r, g, b] = colour.match(/\d+/g).map(Number);
    return { colour, lightness: Math.round((r + g + b) / 3) };
  });

// The vault's copy of the choice, cleared before each pass.
//
// A choice now travels: `interface.theme` reaches the settings file, and a browser with nothing
// saved adopts it. That is the feature, and it makes these two passes dependent — the first one
// ends on Dark, so the second used to open dark on a light machine and the "start" reading stopped
// being about the operating system at all. Reset, so each pass measures what it claims to.
const forget = () =>
  fetch(`${engine.url}/settings/interface?token=${engine.token}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ theme: "system" }),
  });

for (const system of ["dark", "light"]) {
  await forget();
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 860 },
    colorScheme: system,
  });
  const page = await context.newPage();
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
    waitUntil: "domcontentloaded",
  });
  await page.locator("header").waitFor({ timeout: 20000 });
  await page.waitForTimeout(600);

  const started = await bg(page);
  await page.screenshot({ path: `/tmp/shots/theme-${system}-system.png` });

  const button = page.getByRole("button", { name: "Giao diện" });
  const seen = { system: started };
  for (const step of ["light", "dark"]) {
    await button.click();
    await page.waitForTimeout(400);
    seen[step] = await bg(page);
    await page.screenshot({ path: `/tmp/shots/theme-${system}-${step}.png` });
  }

  console.log(
    `system ${system}: start ${seen.system.colour} → light ${seen.light.colour} → dark ${seen.dark.colour}`,
  );

  if (seen.light.lightness < 200) {
    problems.push(`on a ${system} system, choosing Light painted ${seen.light.colour}`);
  }
  if (seen.dark.lightness > 60) {
    problems.push(`on a ${system} system, choosing Dark painted ${seen.dark.colour}`);
  }
  await context.close();
}

// ---- and the choice travels, which is the point of writing it down ---------
//
// A fresh context is a fresh `localStorage`: exactly the state of a second window, another machine
// on the same vault, or a reinstall. Before the settings file was read, every one of those started
// from `system` however many times the user had said otherwise.
{
  await fetch(`${engine.url}/settings/interface?token=${engine.token}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ theme: "dark" }),
  });
  const fresh = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 860 },
    colorScheme: "light",
  });
  const page = await fresh.newPage();
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
    waitUntil: "domcontentloaded",
  });
  await page.locator("header").waitFor({ timeout: 20000 });
  // The settings read happens after first paint on purpose, so this is polled rather than sampled.
  let seen = await bg(page);
  for (let i = 0; i < 20 && seen.lightness > 60; i++) {
    await page.waitForTimeout(250);
    seen = await bg(page);
  }
  console.log(`a new browser on a light machine opened ${seen.colour}`);
  if (seen.lightness > 60) {
    problems.push(`the saved choice did not reach a fresh browser: ${seen.colour}`);
  }
  await fresh.close();
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("theme ok");
