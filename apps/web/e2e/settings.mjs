/**
 * Settings, which is now a place rather than a scroll.
 *
 * It was one narrow column six screenfuls long, with every setting mounted and fetched whether or
 * not anybody was looking at it. This drives the rail: that each section is reachable, that the URL
 * remembers which one is open, and that the section nobody could reach at all before — how long
 * recordings are kept — reads the daemon and writes back to it.
 *
 * The storage panel is the one with something irreversible in it, so the check that matters most
 * here is that pressing "see what can go" does not delete anything.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "settings" });
const { url: appUrl, port, token, home } = engine;

// Audio for the seeded meeting, so the storage panel has something to measure. Bytes, not a real
// recording: what is being tested is the accounting, and the daemon's own tests seed it the same
// way.
if (home) {
  const audio = join(home, "audio/01E2E1");
  mkdirSync(audio, { recursive: true });
  writeFileSync(join(audio, "mic.opus"), Buffer.alloc(3 * 1024 * 1024, 7));
}

const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
});
const page = await context.newPage();
const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
page.on("console", (m) => m.type() === "error" && problems.push(`console: ${m.text()}`));

await page.goto(`${appUrl}?port=${port}&token=${token}#/settings`, { waitUntil: "networkidle" });
await page.getByTestId("settings-nav").waitFor({ timeout: 10_000 });

// Every section is reachable, and each one draws something.
const sections = ["general", "recording", "ai", "translation", "storage", "about"];
for (const id of sections) {
  await page.getByTestId(`settings-tab-${id}`).click();
  await page.waitForTimeout(250);
  const pane = page.getByTestId(`settings-${id}`);
  if (id !== "about" && (await pane.count()) === 0) {
    problems.push(`the ${id} tab did not draw its section`);
    continue;
  }
  const shown = id === "about" ? await page.locator("main").innerText() : await pane.innerText();
  if (shown.trim().length < 20) problems.push(`the ${id} section is empty`);
}

// The URL remembers, so a section can be linked to.
if (!/section=about/.test(page.url())) {
  problems.push(`the section is not in the URL: ${page.url()}`);
}
await page.reload({ waitUntil: "networkidle" });
await page.waitForTimeout(600);
if ((await page.getByTestId("settings-tab-about").getAttribute("aria-current")) !== "page") {
  problems.push("a reload did not come back to the section that was open");
}

// ---- translation: turning it on has to leave you with a model -------------
//
// The setting saved happily, the model was never downloaded, and the first translated line arrived
// as an error in the middle of a meeting. The section now asks the catalogue whether that exact id
// is on the machine and offers to fetch it here — no trip to another screen, and no terminal.
//
// The 611 MB download is not run: `/installs` and `/catalogue` are answered by this suite, because
// what is being checked is that the screen asks for the right model and shows what came back.
{
  await page.goto(`${appUrl}?port=${port}&token=${token}#/settings?section=translation`, {
    waitUntil: "networkidle",
  });
  await page.getByTestId("settings-translation").waitFor({ timeout: 10_000 });

  // The input is `sr-only` and the drawn box sits over it, so the label is what a person clicks
  // and what this clicks too.
  const enable = page.getByRole("checkbox", { name: "Dùng mô hình riêng để dịch" });
  if (!(await enable.isChecked())) {
    await page.getByText("Dùng mô hình riêng để dịch").click();
  }
  await page.waitForTimeout(600);

  const install = page.getByRole("button", { name: /Cài mô hình dịch/ });
  if ((await install.count()) === 0) {
    problems.push("turning translation on offers no way to get the model");
  } else {
    // The size is on the button, because 611 MB is the fact that decides whether now is the moment.
    const label = await install.innerText();
    if (!/\d+\s*(MB|GB)/.test(label)) {
      problems.push(`the install button does not say how big the download is: ${label}`);
    }

    // Pressing it starts a job for the model the field names, and the progress is on screen.
    // `?token=…` is on every request the app makes, so a pattern anchored at the end matches
    // nothing — the same trap that once made four states in `model-list.mjs` test nothing at all.
    let asked = null;
    await page.route(/\/installs(\?|$)/, async (route) => {
      if (route.request().method() === "POST") {
        asked = JSON.parse(route.request().postData() ?? "{}").id;
        await route.fulfill({
          json: { model: asked, name: asked, state: "downloading", done: 30, total: 100 },
        });
        return;
      }
      await route.fulfill({
        json: [{ model: asked, name: asked, state: "downloading", done: 30, total: 100 }],
      });
    });
    await install.click();
    await page.waitForTimeout(800);
    if (asked !== "small100") problems.push(`the wrong model was asked for: ${asked}`);
    if (!(await page.getByTestId("settings-translation").innerText()).includes("30%")) {
      problems.push("a running download shows no progress in the translation section");
    }
    await page.unroute(/\/installs(\?|$)/);
  }

  // And when the model is already there, the section says so instead of offering the download
  // again — the state that used to be indistinguishable from "not installed".
  await page.route(/\/catalogue/, async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    await route.fulfill({
      json: {
        ...body,
        models: (body.models ?? []).map((m) =>
          m.id === "small100" ? { ...m, installed: true } : m,
        ),
      },
    });
  });
  await page.reload({ waitUntil: "networkidle" });
  await page.getByTestId("settings-translation").waitFor({ timeout: 10_000 });
  await page.waitForTimeout(900);
  const ready = await page.getByTestId("settings-translation").innerText();
  if (!ready.includes("Đã cài small100")) {
    problems.push(`an installed translation model is not reported as installed: ${ready}`);
  }
  if ((await page.getByRole("button", { name: /Cài mô hình dịch/ }).count()) > 0) {
    problems.push("a model that is already installed is still offered for download");
  }
  await page.unroute(/\/catalogue/);
  console.log("translation: install offered here, and an installed model is named as installed");
}

// ---- the numbers a recording is made with --------------------------------
//
// How much silence ends a sentence, and how loud counts as speech. Both have been in the settings
// file since the daemon was written, enforced on every session, and reachable only by editing that
// file — so the delay before text appears was a decision made once, by us, for everybody.
{
  await page.goto(`${appUrl}?port=${port}&token=${token}#/settings?section=recording`, {
    waitUntil: "networkidle",
  });
  await page.getByTestId("settings-capture").waitFor({ timeout: 10_000 });

  const silence = page.getByTestId("min-silence");
  await silence.fill("900");
  await silence.dispatchEvent("pointerup");
  await page.waitForTimeout(700);

  const saved = await fetch(`${appUrl}/settings?token=${token}`).then((r) => r.json());
  const ms = saved?.settings?.recording?.min_silence_ms;
  console.log(`trailing silence is now ${ms} ms`);
  if (ms !== 900) problems.push(`the slider did not reach the daemon: ${JSON.stringify(ms)}`);

  // And back, from the button that exists so somebody who moved it can undo that without
  // remembering what it was.
  await page.getByRole("button", { name: "Về mặc định" }).click();
  await page.waitForTimeout(700);
  const back = await fetch(`${appUrl}/settings?token=${token}`).then((r) => r.json());
  if (back?.settings?.recording?.min_silence_ms !== 500) {
    problems.push(`reset left it at ${JSON.stringify(back?.settings?.recording?.min_silence_ms)}`);
  }
}

// ---- storage: the section that had no interface at all --------------------
await page.goto(`${appUrl}?port=${port}&token=${token}#/settings?section=storage`, {
  waitUntil: "networkidle",
});
await page.getByTestId("settings-storage").waitFor({ timeout: 10_000 });
await page.waitForTimeout(800);

const audioUsed = await page.getByTestId("usage-storage.audio").innerText();
console.log(`audio on disk: ${audioUsed}`);
if (!/MB/.test(audioUsed))
  problems.push(`the audio total does not read as megabytes: ${audioUsed}`);

const rows = await page.getByTestId("storage-recording").count();
console.log(`recordings listed: ${rows}`);
if (rows === 0) problems.push("no recording was listed against the space it uses");

// The retention setting: it exists in the settings file, was enforced on every start, and until now
// could only be changed by editing TOML.
const days = page.getByTestId("keep-days");
await days.fill("30");
await days.blur();
await page.waitForTimeout(600);
const saved = await fetch(`${appUrl}/settings?token=${token}`).then((r) => r.json());
console.log(`the daemon now keeps audio for ${saved.settings.storage.audio_retention_days} days`);
if (saved.settings.storage.audio_retention_days !== 30) {
  problems.push(`the setting did not reach the daemon: ${JSON.stringify(saved.settings.storage)}`);
}

// Checking must not delete. The daemon treats a prune with no parameter as a dry run; the screen
// has to ask before it does the real one, because this is the only thing in the app that cannot be
// undone.
await page.getByTestId("plan-prune").click();
await page.getByTestId("prune-plan").waitFor({ timeout: 10_000 });
console.log(`the plan says: ${await page.getByTestId("prune-plan").innerText()}`);
const after = await fetch(`${appUrl}/storage?token=${token}`).then((r) => r.json());
if (after.audio_bytes === 0) problems.push("asking what could go deleted it");
await page.screenshot({ path: "/tmp/shots/settings-storage.png" });

// A phone: the rail becomes a row, and it must not eat the screen.
await page.setViewportSize({ width: 390, height: 780 });
await page.waitForTimeout(400);
const nav = await page.getByTestId("settings-nav").boundingBox();
console.log(`on a phone the rail is ${Math.round(nav?.height ?? 0)}px tall`);
if ((nav?.height ?? 0) > 120) {
  problems.push(`the settings rail takes ${Math.round(nav?.height ?? 0)}px of a 780px screen`);
}
await page.screenshot({ path: "/tmp/shots/settings-narrow.png" });

await browser.close();
engine.stop();
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("settings ok");
