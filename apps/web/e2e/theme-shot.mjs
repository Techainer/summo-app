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

for (const system of ["dark", "light"]) {
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

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("theme ok");
