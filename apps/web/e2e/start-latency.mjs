/**
 * Where the three seconds between pressing record and a running meeting go.
 *
 * Reported as *"vào app bấm start meeting cũng rất chậm"*, and `e2e/microphone.mjs` put a number
 * on it — 2977 ms — without saying what the number was made of. "Starting is slow" is four costs
 * in a trench coat: a voice detector per lane, a decoder per lane, and whichever of them the warm
 * slot already paid for. Which one dominates decides what is worth fixing.
 *
 * Both cases are driven, because they are different questions:
 *
 * 1. **Pressed immediately.** What a user does: open the app, press record. The warm slot is being
 *    filled at that moment or has not been asked for at all.
 * 2. **Pressed after the slot is full.** The case the warm slot was built for, and the measure of
 *    how much it is actually worth.
 *
 * The evidence is the daemon's own log — `warm slot` and `lane ready`, both at `info` — rather
 * than a wall clock around the press, because a wall clock cannot tell a missed slot from a slow
 * one.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { boot, plain } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-fleurs.wav");

const local = await mirror(["gipformer-65m", "silero-vad-v5"], { name: "start-latency" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  process.exit(1);
}

const engine = await boot({ name: "start-latency", seed: false, registry: local.registry });
const at = (path) => `${engine.url}${path}${path.includes("?") ? "&" : "?"}token=${engine.token}`;
const problems = [];

for (const id of ["gipformer-65m", "silero-vad-v5"]) {
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 600; i += 1) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((candidate) => candidate.model === id);
    if (job?.state === "done") break;
    if (job?.state === "failed") {
      console.error(`${id} did not install: ${job.error}`);
      process.exit(1);
    }
    await new Promise((r) => setTimeout(r, 500));
  }
}

if ((await (await fetch(at("/onboarding"))).json()).recognition !== true) {
  console.log("this daemon has no recogniser: nothing to time");
  engine.stop();
  process.exit(0);
}

const browser = await chromium.launch({
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    `--use-file-for-fake-audio-capture=${wav}%noloop`,
  ],
});

async function open() {
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 900 },
    permissions: ["microphone"],
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
    waitUntil: "domcontentloaded",
  });
  await page.locator("header").waitFor({ timeout: 20000 });
  const later = page.getByRole("button", { name: "Để sau" });
  if ((await later.count()) > 0) await later.click();
  return { context, page };
}

/** Press, and wait for a meeting that can be stopped. */
async function timePress(page) {
  const began = Date.now();
  await page
    .getByRole("button", { name: /Bắt đầu ghi/ })
    .first()
    .click();
  await page
    .getByRole("button", { name: /Dừng|Kết thúc/ })
    .first()
    .waitFor({ timeout: 40_000 });
  return Date.now() - began;
}

/** Stop, so the next run starts from a daemon that is not recording. */
async function stop(page) {
  await page
    .getByRole("button", { name: /Dừng|Kết thúc/ })
    .first()
    .click()
    .catch(() => undefined);
  await page.waitForTimeout(1500);
}

/** The `lane ready` and `warm slot` lines the daemon wrote since `from`. */
function breakdown(from) {
  const log = plain(engine).slice(from);
  const lanes = [...log.matchAll(/lane ready.*/g)].map((m) => m[0]);
  const slot = [...log.matchAll(/(warm slot|models\/warm|warming).*/g)].map((m) => m[0]);
  return { lanes, slot, at: plain(engine).length };
}

let mark = plain(engine).length;
const rows = [];

// ---- 1. pressed the moment the app is open --------------------------------
{
  const { context, page } = await open();
  const ms = await timePress(page);
  const { lanes, slot } = breakdown(mark);
  mark = plain(engine).length;
  rows.push({ case: "pressed straight away", ms, lanes, slot });
  await stop(page);
  await context.close();
}

// ---- 1b. pressed after a pause, letting the card's own nudge land ----------
//
// The record card asks the daemon to warm when it opens. A user who reads the screen for a few
// seconds before pressing should therefore get the fast path for free. Whether they do depends on
// where the nudge is sent from and how long it takes, and neither was ever measured.
for (const pause of [2000, 5000]) {
  const { context, page } = await open();
  await page.waitForTimeout(pause);
  const ms = await timePress(page);
  const { lanes, slot } = breakdown(mark);
  mark = plain(engine).length;
  rows.push({ case: `pressed ${pause / 1000} s after opening`, ms, lanes, slot });
  await stop(page);
  await context.close();
}

// ---- 2. pressed once the slot has been filled -----------------------------
//
// The daemon is asked to warm and the answer is waited for, which is what the record card's own
// nudge does — except that this waits, and a user pressing record does not.
{
  const { context, page } = await open();
  const warmed = await (await fetch(at("/models/warm"), { method: "POST" })).json();
  console.log(`warm slot filled ahead of the press: ${JSON.stringify(warmed.ready ?? warmed)}`);
  const ms = await timePress(page);
  const { lanes, slot } = breakdown(mark);
  mark = plain(engine).length;
  rows.push({ case: "warm slot already full", ms, lanes, slot });
  await stop(page);
  await context.close();
}

console.log("");
for (const row of rows) {
  console.log(`${row.case}: ${row.ms} ms from press to a meeting that can be stopped`);
  for (const line of row.slot) console.log(`  ${line.trim()}`);
  for (const line of row.lanes) console.log(`  ${line.trim()}`);
}

await browser.close();
engine.stop();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\nstart latency measured");
