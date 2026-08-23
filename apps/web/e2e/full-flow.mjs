/**
 * The whole product, driven the way a person drives it.
 *
 * Chromium is given a WAV file as its microphone, so this exercises the real path end to end:
 * getUserMedia → the capture worklet's resampling → the WebSocket → the daemon → voice detection →
 * decoding → events → React → the file on disk. Every piece is unit-tested; only this catches them
 * being wired together wrongly.
 *
 * It used to take a URL, a token and a WAV path on the command line, which meant it ran when
 * somebody remembered to run it — and it is the one suite that would notice recognition being
 * broken altogether. Now it boots its own daemon, installs a speech model from the local mirror the
 * other suites already use, and records `fixtures/vi-fleurs.wav`, so it belongs in `pnpm e2e` with
 * everything else.
 *
 * What it asserts is that *text arrives*, never which words. Asserting the words would turn every
 * model change into a broken test, and the model is allowed to change.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-fleurs.wav");

// The models this needs, served from this machine: a recogniser and a voice detector. Without the
// detector there are no utterance boundaries and nothing is ever committed to the transcript.
const local = await mirror(["gipformer-65m", "silero-vad-v5"], { name: "full-flow" });

// No skipping here, unlike `models.mjs`. That suite can check a catalogue screen without installing
// anything; this one is audio going in and text coming out, and there is no version of it worth
// running without the model that does the recognising. If the bytes are not here, say why and stop
// rather than pass having transcribed nothing.
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("the whole-product run needs these models; nothing about it is meaningful without");
  process.exit(1);
}
// `local.registry`, not `local`. Passing the whole object set `SUMMO_REGISTRY` to
// "[object Object]", the daemon fell back to the published registry, and this suite quietly
// downloaded its models from github.com on every run — the exact dependency `mirror.mjs` exists to
// remove, in the one suite that most needs it. It passed for as long as GitHub felt like answering.
const engine = await boot(process.argv, { name: "full-flow", registry: local.registry });
const { url: appUrl, port, token } = engine;

/** Install a model through the daemon, and wait for it. */
async function install(id) {
  const at = (path) => `${appUrl}${path}${path.includes("?") ? "&" : "?"}token=${token}`;
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 300; i++) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((candidate) => candidate.model === id);
    if (job?.state === "done") return;
    if (job?.state === "failed") throw new Error(`${id}: ${job.error ?? "install failed"}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`${id}: still installing after 150s`);
}

for (const id of ["silero-vad-v5", "gipformer-65m"]) {
  console.log(`installing ${id}…`);
  await install(id);
}

const browser = await chromium.launch({
  args: [
    "--use-fake-ui-for-media-stream",
    "--use-fake-device-for-media-stream",
    `--use-file-for-fake-audio-capture=${wav}`,
    "--autoplay-policy=no-user-gesture-required",
  ],
});
// The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
// this the app honours the machine's locale — which is exactly what it should do, and which made
// every assertion here fail the moment translation landed.
const context = await browser.newContext({
  locale: "vi-VN",
  permissions: ["microphone"],
  viewport: { width: 1180, height: 760 },
  colorScheme: "dark",
});
const page = await context.newPage();

const problems = [];
page.on("console", (m) => {
  if (m.type() === "error") problems.push(`console: ${m.text()}`);
});
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });

console.log("clicking record…");
// The header carries a record button as well as the home card, and both are the real control.
// This drives the one on the screen a person is looking at.
await page
  .getByTestId("home")
  .getByRole("button", { name: /Bắt đầu ghi/ })
  .click();

// Wait for the first committed line rather than a fixed sleep: the assertion is that text arrives,
// and a timeout here is the failure worth reporting.
// Two minutes, not one: this runs against a debug build of the daemon, where the first decode also
// pays for loading the model, and a flake here would be read as "recognition is broken".
await page
  .locator('[data-testid="transcript-line"]')
  .first()
  .waitFor({ timeout: 120000 })
  .catch((error) => {
    console.log("--- daemon log ---\n" + engine.log().slice(-4000));
    throw error;
  });
await page.waitForTimeout(12000);

const lines = await page.locator('[data-testid="transcript-line"]').allInnerTexts();
await page.screenshot({ path: "/tmp/shots/recording.png" });

// The compact window is what sits on top of a call, so check it renders while recording.
await page
  .getByRole("button", { name: /Thu gọn/ })
  .first()
  .click();
await page.waitForTimeout(300);
await page.screenshot({ path: "/tmp/shots/compact.png" });
await page
  .getByRole("button", { name: /Mở rộng/ })
  .first()
  .click();

// ---- changing the language mid-meeting -----------------------------------
//
// The part a default cannot do. A preference is right most of the time and wrong exactly when it
// matters — the call that turned out to be in English — and the old answer was to stop, change a
// setting and start again, which costs the part of the meeting where somebody noticed.
{
  const at = (path) => `${appUrl}${path}${path.includes("?") ? "&" : "?"}token=${token}`;
  const before = await (await fetch(at("/status"))).json();
  if (before.state !== "recording")
    problems.push(`not recording before the change: ${before.state}`);

  // Exactly one, and inside the meeting's own bar.
  //
  // These controls used to live only in the shell's banner, which is drawn *above* the page — so
  // the settings for the meeting sat outside the meeting, and that banner has a dismiss button, so
  // one click removed the only way to change the model, the language or the translation for the
  // rest of the recording. `LiveBar` carries them now and `ListeningIn` yields on this page.
  //
  // The count is asserted because the obvious way to implement that is to render both, and an
  // earlier attempt did exactly that: two buttons named "Đổi", ambiguous to a reader and fatal to
  // this selector.
  const changers = page.getByRole("button", { name: "Đổi", exact: true });
  if ((await changers.count()) !== 1) {
    problems.push(`expected one "Đổi" on the meeting page, found ${await changers.count()}`);
  }
  if ((await page.getByTestId("live-bar").getByRole("button", { name: "Đổi" }).count()) !== 1) {
    problems.push("the in-meeting controls are not inside the recording bar");
  }

  await changers.first().click();
  await page.screenshot({ path: "/tmp/shots/in-meeting-config.png" });
  await page.getByLabel("Ngôn ngữ nói").selectOption("vi");
  await page.waitForTimeout(3000);

  const after = await (await fetch(at("/status"))).json();
  console.log(
    `language mid-meeting: ${before.language ?? "(model's own)"} → ${after.language}, ` +
      `segments ${before.segments} → ${after.segments}, still ${after.state}`,
  );
  if (after.state !== "recording") {
    problems.push(`the meeting ended when the language changed: ${JSON.stringify(after)}`);
  }
  if (after.language !== "vi") {
    problems.push(`the daemon did not take the new language: ${JSON.stringify(after)}`);
  }
  // Nothing already transcribed may be lost: the count only ever goes up.
  if (after.segments < before.segments) {
    problems.push(`segments went backwards: ${before.segments} → ${after.segments}`);
  }
}

console.log("clicking stop…");
await page
  .getByRole("button", { name: /Dừng ghi/ })
  .first()
  .click();
await page.waitForTimeout(3000);
const notice = await page
  .locator(".notice")
  .innerText()
  .catch(() => "");
await page.screenshot({ path: "/tmp/shots/stopped.png" });

await browser.close();
await engine.stop();

console.log(`\ntranscript lines on screen: ${lines.length}`);
for (const line of lines.slice(0, 8)) console.log(`  ${line}`);
console.log(`\nstatus bar after stop: ${notice}`);
console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nno console errors");

if (lines.length === 0) {
  console.log("\nFAIL: no transcript reached the screen");
  process.exit(1);
}
