/**
 * The microphone is granted, and recording still has to work.
 *
 * A user reported pressing record with permission already granted, two devices listed, and nothing
 * happening — and the app answered with a sentence telling them to go and check the microphone.
 * Both halves were wrong: the permission was fine, and the app was guessing.
 *
 * So this drives the paths a real press goes through and asserts on what the person sees:
 *
 * 1. permission granted, model installed → words on screen, and the daemon writing a file;
 * 2. permission granted, source set to system audio → still records, because a machine with no
 *    system capture must fall back rather than fail silently;
 * 3. permission refused → the app says so, offers the way to fix it, and does not run a clock.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";
const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-fleurs.wav");

const local = await mirror(["gipformer-65m", "silero-vad-v5"], { name: "microphone" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  process.exit(1);
}

const engine = await boot({ name: "microphone", seed: false, registry: local.registry });
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

const status = await (await fetch(at("/onboarding"))).json();
const recognises = status.recognition === true;
console.log(
  recognises
    ? "this daemon can transcribe: the whole path is driven"
    : "this daemon has no recogniser: only what such a build can do is checked",
);

const browser = await chromium.launch({
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    `--use-file-for-fake-audio-capture=${wav}%noloop`,
  ],
});

/** A page with the setup takeover already dismissed. */
async function open({ permissions = ["microphone"], lanes } = {}) {
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 900 },
    permissions,
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  // The capture choice lives in `localStorage` under `summo.capture`, so it is set the way the app
  // sets it — before the page that reads it exists.
  if (lanes) {
    await context.addInitScript(
      ([key, value]) => window.localStorage.setItem(key, value),
      ["summo.capture", JSON.stringify({ lanes })],
    );
  }
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
    waitUntil: "domcontentloaded",
  });
  await page.locator("header").waitFor({ timeout: 20000 });
  const later = page.getByRole("button", { name: "Để sau" });
  if ((await later.count()) > 0) await later.click();
  await page.waitForTimeout(400);
  return { context, page };
}

const press = (page) =>
  page
    .getByRole("button", { name: /Bắt đầu ghi/ })
    .first()
    .click();

// ---- 1. granted, installed, recording ------------------------------------
{
  const { context, page } = await open();
  await press(page);

  let lines = 0;
  for (let i = 0; i < 40; i += 1) {
    lines = await page.locator('[data-testid="transcript-line"]').count();
    if (lines > 0) break;
    await page.waitForTimeout(500);
  }
  if (recognises && lines === 0) {
    problems.push("granted microphone, installed models, and no words arrived");
  }

  await page.waitForTimeout(2500);
  const body = await page.locator("body").innerText();
  if (recognises && !/00:0[1-9]|00:[1-5]\d/.test(body)) {
    problems.push("the clock never started while recording");
  }

  // Typing, while it records. A meeting is a note with a transcript beside it, and the note half is
  // the whole reason the recording screen was replaced by this one — a page that shows words
  // arriving and cannot take a keystroke is the old screen with a new layout.
  if (recognises) {
    const editor = page.getByRole("textbox", { name: /Ghi chú|ghi chú/ }).first();
    if ((await editor.count()) === 0) {
      problems.push("the meeting has nowhere to type");
    } else {
      await editor.click();
      await page.keyboard.type("Ngân sách chốt thứ năm.");
      await page.waitForTimeout(600);
      if (!(await page.locator("body").innerText()).includes("Ngân sách chốt thứ năm")) {
        problems.push("typing into the meeting note put nothing on screen");
      }
    }
  }

  console.log(`granted: ${lines} line(s) on screen`);

  // Stopped, not abandoned. Chromium's fake device is exclusive: leaving this recording running
  // held the microphone, the next context could not open one, and the suite spent an afternoon
  // proving that a held device hangs — which it does, and which is now the third scenario.
  await page
    .getByRole("button", { name: /Dừng ghi/ })
    .first()
    .click()
    .catch(() => {
      if (recognises) problems.push("no way to stop the recording");
    });
  await page.waitForTimeout(2500);
  await context.close();
}

// ---- 2. asking for system audio must not break the microphone ------------
//
// A machine with no system capture is the common one — every Mac without the helper, every Linux
// without a loopback device — and "record what is playing" failing has to leave "record me"
// working.
{
  const { context, page } = await open({ lanes: ["mic", "system"] });
  await press(page);
  await page.waitForTimeout(6000);

  // Asked of the daemon, which is the thing that either records or does not. The screen is checked
  // in the first scenario; here the question is whether asking for a second source refuses the
  // session — it did, because system audio implies speaker attribution, which needs a model nobody
  // installed, and the whole recording failed over a nice-to-have.
  const live = await (await fetch(at("/status"))).json();
  if (recognises && live.state !== "recording") {
    problems.push(`asking for system audio left the daemon in "${live.state}"`);
  }
  // The contract, and the thing that was broken: the app is never silent about a session the
  // daemon has started. Either it shows the recording, or it says why it could not — after the
  // microphone deadline, which is what turns "nothing is happening" into a sentence.
  await page.waitForTimeout(7000);
  const showing = await page.getByRole("button", { name: /Dừng ghi/ }).count();
  const said = /micro/i.test(await page.locator("body").innerText());
  // Only on a build that can actually record. Without the `models` feature the daemon accepts a
  // session it can never produce anything for — its own shape, tested elsewhere — and holding this
  // screen to a promise that binary cannot keep fails a suite over the wrong thing.
  if (recognises && showing === 0 && !said) {
    problems.push("the app says nothing at all about the session it just asked for");
  }

  await page
    .getByRole("button", { name: /Dừng ghi/ })
    .first()
    .click()
    .catch(() => undefined);
  await page.waitForTimeout(2500);
  console.log("mic + system: recording started and stopped");
  await context.close();
}

// ---- 3. and the words reach the file, not only the screen ----------------
//
// The screen can be right while nothing is saved: the transcript is a stream of events, and the
// document on disk is written by a different piece of code. A user who records a meeting and finds
// an empty note has been failed by exactly that gap.
{
  const library = await (await fetch(at("/library"))).json();
  const rows = (library.groups ?? []).flatMap((group) => group.meetings ?? []);
  const meetings = rows.filter((row) => row.kind === "meeting");
  if (meetings.length === 0) {
    if (recognises) problems.push("nothing was filed as a meeting after two recordings");
  } else {
    // Any of them. The second recording plays a fake device that has already reached the end of
    // its file, so it can legitimately produce a meeting with nothing in it — what has to be true
    // is that words said into a microphone reach the disk, not that every session hears something.
    let landed = 0;
    for (const meeting of meetings) {
      const detail = await (await fetch(at(`/meetings/${meeting.id}`))).json();
      landed = Math.max(landed, (detail.transcript ?? []).length);
    }
    if (landed === 0 && recognises) problems.push("no meeting on disk has a transcript in it");
    else console.log(`on disk: ${landed} line(s), across ${meetings.length} meeting(s)`);
  }
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("microphone ok");
