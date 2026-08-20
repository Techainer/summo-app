/**
 * Every page, in the order a person meets them, photographed for review.
 *
 * Not an assertion suite — `shots.mjs` already audits contrast and overflow. This exists because
 * the failures that reached a user were things a screenshot shows and a selector does not: two
 * banners saying the same thing, an instruction pointing at a screen that is not in the sidebar, a
 * theme button that changed an attribute and no colours, a recording screen with nowhere to type.
 *
 * It walks a real flow on a real vault: install from the catalogue, press record, watch the words
 * arrive, stop, and then visit each screen the sidebar offers.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-fleurs.wav");
const OUT = "/tmp/shots/review";

const local = await mirror(["gipformer-65m", "silero-vad-v5"], { name: "review" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  process.exit(1);
}

const engine = await boot({
  name: "review",
  seed: false,
  onboarded: false,
  registry: local.registry,
});
const browser = await chromium.launch({
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    `--use-file-for-fake-audio-capture=${wav}%noloop`,
  ],
});
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
  permissions: ["microphone"],
});
const page = await context.newPage();
const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") problems.push(`console: ${m.text()}`);
});

let step = 0;
const shot = async (name) => {
  step += 1;
  const file = `${OUT}/${String(step).padStart(2, "0")}-${name}.png`;
  await page.screenshot({ path: file, fullPage: false });
  console.log(`  ${file}`);
};

await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
  waitUntil: "domcontentloaded",
});
await page.locator("header").waitFor({ timeout: 20000 });
await page.waitForTimeout(1500);

// ---- 1. the first screen, and installing from it -------------------------
await shot("setup");

const install = page.getByRole("button", { name: "Tải và cài" });
if ((await install.count()) > 0) {
  await install.click();
  // Both halves land: the recogniser, then the detector.
  for (let i = 0; i < 240; i += 1) {
    const jobs = await (await fetch(`${engine.url}/installs?token=${engine.token}`)).json();
    const done = jobs.filter((job) => job.state === "done").map((job) => job.model);
    if (done.includes("gipformer-65m") && done.includes("silero-vad-v5")) break;
    await page.waitForTimeout(500);
  }
  await page.waitForTimeout(1200);
  await shot("setup-installed");
} else {
  problems.push("setup offered nothing to install");
}

const start = page.getByRole("button", { name: "Bắt đầu" });
if ((await start.count()) > 0) await start.first().click();
await page.waitForTimeout(1500);
await shot("home");

// ---- 2. record: the screen a meeting actually happens on -----------------
await page
  .getByRole("button", { name: /Bắt đầu ghi|^Ghi$/ })
  .first()
  .click();
await page.waitForTimeout(9000);
await shot("recording");

const body = await page.locator("body").innerText();
if (!/00:0[1-9]|00:1\d/.test(body)) problems.push("no clock while recording");
const url = page.url();
if (!/pages\//.test(url)) problems.push(`recording did not open a note: ${url}`);

await page
  .getByRole("button", { name: /Dừng ghi/ })
  .first()
  .click()
  .catch(() => problems.push("no stop button"));
await page.waitForTimeout(2500);
await shot("after-stop");

// What the daemon says about the document that was just recorded — the answer the page renders
// from, printed so a run that looks wrong can be read rather than guessed at.
{
  const id = /pages\/([^/?#]+)/.exec(url)?.[1];
  if (id) {
    const detail = await (await fetch(`${engine.url}/meetings/${id}?token=${engine.token}`)).json();
    console.log(
      `  recorded page: kind=${detail.summary?.kind} transcript=${(detail.transcript ?? []).length} sections=[${(detail.sections ?? []).map((s) => s.heading).join("|")}]`,
    );
    console.log(`  file: ${detail.summary?.file}`);
  }
}

// ---- 3. everything the sidebar offers ------------------------------------
for (const [name, label] of [
  ["home", "Trang chính"],
  ["library", "Đã lưu"],
  ["tasks", "Việc"],
  ["agenda", "Lịch"],
  ["analytics", "Thống kê"],
  ["settings", "Cài đặt"],
  ["help", "Trợ giúp"],
]) {
  const item = page
    .getByRole("navigation", { name: "Màn hình" })
    .getByRole("button", { name: label });
  if ((await item.count()) === 0) {
    problems.push(`the sidebar has no ${label}`);
    continue;
  }
  await item.first().click();
  await page.waitForTimeout(1200);
  await shot(name);
}

// The catalogue, which is where the app keeps telling people to go.
await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/models`, {
  waitUntil: "domcontentloaded",
});
await page.waitForTimeout(1500);
await shot("models");

// ---- 4. the same app in light ------------------------------------------
await page.getByRole("button", { name: "Giao diện" }).click();
await page.waitForTimeout(600);
await shot("models-light");
await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/`, {
  waitUntil: "domcontentloaded",
});
await page.waitForTimeout(1500);
await shot("home-light");

await browser.close();
await engine.stop();

console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nno console errors");
