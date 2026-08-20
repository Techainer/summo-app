/**
 * A vault with a recogniser and no voice detector.
 *
 * This is what a user actually had. They installed Whisper from the catalogue, setup said the app
 * was ready, they pressed record — and the app started a clock, turned the button red, and
 * transcribed nothing, for as long as they were willing to watch. The pipeline had refused at
 * `resolve_vad` in the first fifty milliseconds: without a voice detector nothing decides where an
 * utterance ends, so nothing is ever committed to a transcript.
 *
 * Three separate failures made that possible, and this pins all three:
 *
 * 1. the checklist counted speech models only, so it said ready;
 * 2. the refusal arrived before the microphone was open, and the app dropped it on the floor —
 *    the handler was guarded on `state.recording`, which was still false;
 * 3. the clock was started by the button rather than by the daemon.
 */
import { chromium } from "playwright";

import { boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const local = await mirror(["gipformer-65m"], { name: "no-vad" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this suite needs a recogniser to install; without it there is nothing to prove");
  process.exit(1);
}

const engine = await boot({ name: "no-vad", seed: false, registry: local.registry });
const browser = await chromium.launch({
  args: ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"],
});
const problems = [];

const at = (path) => `${engine.url}${path}${path.includes("?") ? "&" : "?"}token=${engine.token}`;

// The recogniser, and deliberately nothing else.
await fetch(at("/installs"), {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ id: "gipformer-65m" }),
});
for (let i = 0; i < 300; i += 1) {
  const jobs = await (await fetch(at("/installs"))).json();
  const job = jobs.find((candidate) => candidate.model === "gipformer-65m");
  if (job?.state === "done") break;
  if (job?.state === "failed") {
    console.error(`the recogniser did not install: ${job.error}`);
    process.exit(1);
  }
  await new Promise((r) => setTimeout(r, 500));
}

// ---- the daemon must not call this ready -------------------------------
{
  const status = await (await fetch(at("/onboarding"))).json();
  const models = status.checks.find((check) => check.step === "models");
  if (status.recognition) {
    if (models.ready) {
      problems.push("a vault with no voice detector was reported as ready to record");
    }
    if (!String(models.detail).includes("dò giọng")) {
      problems.push(`the missing half is not named: ${models.detail}`);
    }
    if (status.can_record) problems.push("`can_record` is true with no voice detector installed");
  }
}

// ---- and the app must say so rather than pretend ------------------------
{
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

  // Past the setup takeover, which is doing its job here — this is about what happens *after*
  // somebody presses record anyway.
  const later = page.getByRole("button", { name: "Để sau" });
  if ((await later.count()) > 0) await later.click();
  await page.waitForTimeout(500);

  await page
    .getByRole("button", { name: /Bắt đầu ghi/ })
    .first()
    .click();
  await page.waitForTimeout(6000);

  const text = await page.locator("body").innerText();
  if (!/dò giọng/.test(text)) {
    problems.push("nothing on screen said a voice detector was missing");
  }

  // The clock. A running timer over a session the daemon refused is the part that made this look
  // like a working recording for seventeen seconds.
  const clock = text.match(/\b00:(\d{2})\b/);
  if (clock && Number(clock[1]) > 1) {
    problems.push(`the timer ran to 00:${clock[1]} over a session that was refused`);
  }

  const recording = await page.getByRole("button", { name: /Dừng ghi/ }).count();
  if (recording > 0) problems.push("the app still claims to be recording");

  await page.screenshot({ path: "/tmp/shots/no-vad.png", fullPage: true });
  await context.close();
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("no vad ok: refused, said why, and the clock stayed put");
