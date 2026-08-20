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

// Exactly "Bắt đầu", which is the button that finishes setup. Without `exact` this also matches
// "Bắt đầu ghi" — the record button in the header, which comes first in the DOM — so the run
// pressed record while the takeover was still up, photographed the setup screen, and filed it as
// the home screen. Three reviews looked at that image and none of us noticed.
const start = page.getByRole("button", { name: "Bắt đầu", exact: true });
if ((await start.count()) === 0) problems.push("setup has no way to finish");
else await start.first().click();
await page.waitForTimeout(1500);
const landed = await page.locator("body").innerText();
if (/Chào mừng tới Summo/.test(landed)) {
  problems.push("finishing setup left the setup screen on screen");
}
// The banner that says a recogniser is missing, over a home screen reached by installing one. The
// status on hand was fetched before the download and the poll that replaces it runs every five
// seconds, so this was true for a few seconds after every first run.
if (/cần để ghi được/.test(landed)) {
  problems.push("the home screen asks for the model that was just installed");
}
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

const at = (hash) => `${engine.url}?port=${engine.port}&token=${engine.token}#${hash}`;

// The catalogue, which is where the app keeps telling people to go — listed, narrowed, and with a
// card opened, because those are three different layouts and only the first was ever looked at.
await page.goto(at("/models"), { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await shot("models");

await page.getByTestId("model-search").fill("whisper");
await page.waitForTimeout(600);
await shot("models-search");
await page.getByTestId("model-search").fill("");
await page.waitForTimeout(400);

const details = page.getByRole("button", { name: "Chi tiết" }).first();
if ((await details.count()) > 0) {
  const card = page.locator("article", { has: page.getByRole("button", { name: "Thu gọn" }) });
  await details.click();
  // Until it settles, not for a fixed second. The daemon fetches the publisher's README to build
  // this page, so on a machine that cannot reach the registry it used to say "Đang tải…" and go on
  // saying it — the app now gives up after eight seconds and says what it knows instead.
  let settled = false;
  for (let i = 0; i < 24; i += 1) {
    if (!(await card.first().innerText()).includes("Đang tải")) {
      settled = true;
      break;
    }
    await page.waitForTimeout(500);
  }
  if (!settled) problems.push("a model's details never stopped loading");
  await shot("models-detail");
} else {
  problems.push("no model card offers its details");
}

// The two settings sections with something to look at: where recordings are kept, and the
// translation model — which is now installed from that section rather than pointed at from it.
for (const [name, section] of [
  ["settings-translation", "translation"],
  ["settings-storage", "storage"],
]) {
  await page.goto(at(`/settings?section=${section}`), { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  await shot(name);
}

// ---- 4. the same app in light ------------------------------------------
await page.goto(at("/models"), { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1200);
await page.getByRole("button", { name: "Giao diện" }).click();
await page.waitForTimeout(600);
await shot("models-light");
await page.goto(at("/"), { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await shot("home-light");
await page.goto(at("/analytics"), { waitUntil: "domcontentloaded" });
await page.waitForTimeout(1500);
await shot("analytics-light");

// ---- 5. and on a phone ---------------------------------------------------
//
// Not a supported target so much as an unavoidable one: the desktop window resizes, and every
// layout here has a point at which it stops working. 390px is where that shows up first.
await page.getByRole("button", { name: "Giao diện" }).click();
await page.waitForTimeout(400);
await page.setViewportSize({ width: 390, height: 780 });
for (const [name, hash] of [
  ["narrow-home", "/"],
  ["narrow-models", "/models"],
  ["narrow-analytics", "/analytics"],
  ["narrow-library", "/library"],
]) {
  await page.goto(at(hash), { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1400);
  await shot(name);
}

await browser.close();
await engine.stop();

console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nno console errors");
