/**
 * Two speech models on one meeting, chosen per sentence.
 *
 * `refine_model` was a setting that did nothing. The models screen has offered "use for refine"
 * since it had buttons, `/settings/models` accepted the role and wrote it to disk, and no session
 * ever read it — `HybridSession` was written, tested, exported and never constructed. So this suite
 * exists to prove the wiring end to end rather than in unit tests that would have passed the whole
 * time it was disconnected.
 *
 * What it drives, with real models and real audio:
 *
 * - a session started with nothing named picks up **both** models from the settings file,
 * - the daemon reports the pair on `/status`, which is what an interface can see,
 * - Vietnamese speech decoded live by Whisper is **revised** by Gipformer, in place, without the
 *   recording stopping — the revision is the whole feature and the only proof the second decoder
 *   ran at all,
 * - the same model in both roles does not refuse the recording.
 *
 * The routing decision — refine only when the second model claims the language the first one heard
 * — is unit-tested in `refine.rs`, because provoking an English sentence out of a Vietnamese
 * fixture is not something a suite should try to arrange.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-fleurs.wav");
const problems = [];

// Whisper hears ninety-nine languages badly and reports which one it heard; Gipformer hears
// Vietnamese and nothing else, accurately. That asymmetry is the entire reason for the feature.
const MODELS = ["whisper-tiny", "gipformer-65m", "silero-vad-v5"];
const local = await mirror(MODELS, { name: "bilingual" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this suite is about two models disagreeing; it means nothing without both");
  process.exit(1);
}

const engine = await boot(process.argv, { name: "bilingual", registry: local.registry });
const { url: appUrl, port, token } = engine;
const at = (path) => `${appUrl}${path}${path.includes("?") ? "&" : "?"}token=${token}`;
const status = async () => (await fetch(at("/status"))).json();

async function install(id) {
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 600; i++) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((candidate) => candidate.model === id);
    if (job?.state === "done") return;
    if (job?.state === "failed") throw new Error(`${id}: ${job.error ?? "install failed"}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`${id}: still installing after 300s`);
}

for (const id of MODELS) {
  console.log(`installing ${id}…`);
  await install(id);
}

const pick = (role, model) =>
  fetch(at("/settings/models"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ role, model }),
  });

// Exactly what pressing the two buttons on the models screen writes. Nothing here reaches into the
// session — the point is that the *settings file* is enough, which is the half that was missing.
await pick("live", "whisper-tiny");

// ---- the same model twice must not break recording -------------------------
//
// Reachable by pressing "use" and then "use for refine" on one row, and `SessionSpec::validate`
// refuses the pair outright — so without the guard this is a record button that fails.
{
  await pick("refine", "whisper-tiny");
  // `/catalogue` is what the models screen reads to draw which row is chosen for what, so this is
  // the same answer a user would see rather than a private corner of the settings file.
  const { chosen } = await (await fetch(at("/catalogue"))).json();
  if (chosen?.refine !== "whisper-tiny") {
    problems.push(`the models screen would not show the second role: ${JSON.stringify(chosen)}`);
  }
}

// And the pair the rest of this file is about, written last so the block above cannot decide it.
await pick("refine", "gipformer-65m");

const browser = await chromium.launch({
  args: [
    "--use-fake-ui-for-media-stream",
    "--use-fake-device-for-media-stream",
    `--use-file-for-fake-audio-capture=${wav}`,
    "--autoplay-policy=no-user-gesture-required",
  ],
});
const context = await browser.newContext({
  locale: "vi-VN",
  permissions: ["microphone"],
  viewport: { width: 1180, height: 820 },
  colorScheme: "dark",
});
const page = await context.newPage();
page.on("console", (message) => {
  if (message.type() === "error") problems.push(`console: ${message.text()}`);
});

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });

console.log("clicking record…");
await page
  .getByTestId("home")
  .getByRole("button", { name: /Bắt đầu ghi/ })
  .click();

const firstLine = page.locator('[data-testid="transcript-line"]').first();
await firstLine.waitFor({ timeout: 120000 }).catch((error) => {
  console.log("--- daemon log ---\n" + engine.log().slice(-4000));
  throw error;
});

// ---- the daemon is running both, and says so -------------------------------
{
  const now = await status();
  console.log(`live ${now.live_model}, refine ${now.refine_model}, ${now.state}`);
  if (now.live_model !== "whisper-tiny") {
    problems.push(`the live model came from nowhere useful: ${now.live_model}`);
  }
  if (now.refine_model !== "gipformer-65m") {
    problems.push(
      `the refine model in the settings never reached the session: ${JSON.stringify(now.refine_model)}`,
    );
  }
}

// ---- a line gets a better version of itself, mid-recording -----------------
//
// The only evidence the second decoder ran. Asserted against the daemon's own log rather than
// against the screen: text on screen also changes when a partial becomes a final, so watching it
// grow proves nothing about refinement — an earlier version of this suite passed on exactly that
// and would have gone on passing with the feature disconnected.
{
  let refined = false;
  for (let i = 0; i < 160 && !refined; i++) {
    await page.waitForTimeout(500);
    refined = engine.log().includes("refined an utterance");
  }

  if (!refined) {
    console.log("--- daemon log ---\n" + engine.log().slice(-3000));
    problems.push("the second model never revised anything — the refine pass did not run");
  } else {
    // And the revision reaches the screen, which the log cannot say. `Event::Revise` travels the
    // same socket as everything else and the reducer only accepts it over a `final`
    // (`accepts` in `protocol.ts`); a rule that said otherwise would drop every revision silently,
    // with the daemon still logging that it had made one.
    const revised = page.locator('[data-testid="transcript-line"][data-source="revised"]');
    const shown = await revised
      .first()
      .waitFor({ timeout: 30000 })
      .then(() => true)
      .catch(() => false);
    if (!shown) {
      problems.push("the daemon revised a line and the transcript still shows the first version");
    } else {
      const line = await revised
        .first()
        .innerText()
        .catch(() => "");
      console.log(`refined and on screen: ${JSON.stringify(line.slice(0, 60))}`);
    }
  }

  const during = await status();
  if (during.state !== "recording") {
    problems.push("the meeting ended while the second model was working");
  }
  await page.screenshot({ path: "/tmp/shots/bilingual.png" });
}

await page
  .getByRole("button", { name: /Dừng ghi/ })
  .first()
  .click();
await page.waitForTimeout(2000);

await browser.close();
await engine.stop();

console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nbilingual ok");
if (problems.length) process.exit(1);
