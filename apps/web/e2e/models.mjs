/**
 * The model catalogue, on screen.
 *
 * The registry has always been able to answer this and nothing ever asked it — the only way to
 * install a model that was not the recommended one was `summo pull` on a command line. This checks
 * the screen that fixes that, and specifically the two things a card has to say *before* somebody
 * spends several hundred megabytes: how big it is, and whether the licence means the download goes
 * somewhere other than us.
 *
 * Points at the local registry directory, so the suite does not depend on a deployed one.
 */
import { chromium } from "playwright";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { boot } from "./daemon.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REGISTRY = join(HERE, "../../../../summo-registry");

const problems = [];
const fail = (message) => problems.push(message);

const engine = await boot({ name: "models" });
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
  colorScheme: "dark",
});
const page = await context.newPage();

try {
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/models`, {
    waitUntil: "networkidle",
  });
  await page.locator('[data-testid="models"]').waitFor({ timeout: 10000 });
  await page.waitForTimeout(800);

  const body = await page.locator('[data-testid="models"]').innerText();

  // Grouped by what the model does. "Which speech model" and "which translator" are different
  // questions asked at different times.
  // Case-insensitively: the headings are uppercased in CSS, and `innerText` reports what is
  // rendered. Matching the source casing passed by accident against the intro paragraph, which
  // happens to contain the same words.
  const shouty = body.toLocaleUpperCase("vi");
  for (const heading of ["Nhận dạng giọng nói", "Dịch"]) {
    if (!shouty.includes(heading.toLocaleUpperCase("vi"))) fail(`no section for ${heading}`);
  }

  // Every model the registry knows, not only the speech ones the setup screen offers.
  for (const id of ["gipformer-65m", "small100", "silero-vad-v5", "campplus-sv"]) {
    if (!body.includes(id)) fail(`${id} is missing from the catalogue`);
  }

  // Size before you commit to it.
  if (!/\d+\s*MB|\d+(\.\d+)?\s*GB/.test(body)) {
    fail("no download size on any card");
  }

  // The licence, and the flag that says the bytes come from somewhere other than us. Finding that
  // out at the download is finding out after committing.
  if (!body.includes("MIT")) fail("no licence shown");
  if (!body.includes("upstream")) {
    fail("a model Summo does not host is not marked as coming from upstream");
  }

  const install = page.getByRole("button", { name: "Cài", exact: true });
  if ((await install.count()) === 0) fail("nothing can be installed from this screen");

  await page.screenshot({ path: "/tmp/shots/models.png", fullPage: true });

  // An unreachable registry is a state, not a blank screen: this is an app expected to work on a
  // plane. Simulated by refusing the request the catalogue makes.
  await page.route("**/catalogue*", (route) => route.abort());
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(600);
  const offline = await page.locator("body").innerText();
  if (!offline.includes("kho mô hình")) {
    fail("with the catalogue unreachable the screen says nothing about why it is short");
  }
} finally {
  await browser.close();
  engine.stop();
}

if (problems.length) {
  for (const problem of problems) console.error(`FAIL ${problem}`);
  process.exit(1);
}
console.log("models ok");
