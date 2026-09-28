/**
 * The declaration reaches the model that acts on it.
 *
 * Background, from a real meeting: Vietnamese and English both declared, `whisper-tiny`
 * listening, `gipformer` installed. Whisper labelled Vietnamese speech `zh`, `he`, `ja` and `ru`.
 * Routing asked the label which specialist claimed that language, nothing claimed `zh`, and the
 * rescue path refused every one of those lines — locked shut by exactly the guess it exists to
 * correct. The transcript kept `大家看今天今天今天…` twenty-four times over, and then translated it.
 *
 * The rule that fixes it — a language the *user declared* summons its specialist whatever the
 * label says — is unit-tested in `refine.rs`, and that is where it has to be tested: reproducing
 * it end to end needs audio the fast model gets **wrong**, and which clip a model mislabels is not
 * something a suite can arrange. This was written against `vi-fleurs.wav` and confirmed to pass
 * both with and without the fix, because whisper-tiny labels that clip correctly. Saying so here
 * rather than leaving a green suite that looks like a guard and is not.
 *
 * What it does guard is the other half, and the half a unit test cannot reach: that the
 * declaration travels at all. It has to get from the browser's `summo.capture`, through
 * `session_start`, through `resolve_models`, into `Refiner::declared` — four places it could be
 * dropped with every unit test still passing. So this drives real Vietnamese audio with two
 * languages declared and asserts the daemon's own log says the specialist ran.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { boot, plain } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-fleurs.wav");

// `whisper-tiny`, deliberately. It is the model a fresh install is most likely to have and the one
// that produced the report; a suite that only drives `whisper-base` would not reproduce the
// mislabelling that breaks the routing.
const LIVE = "whisper-tiny";
const SPECIALIST = "gipformer-1.5-68m";
const MODELS = [LIVE, SPECIALIST, "silero-vad-v5"];

const local = await mirror(MODELS, { name: "mislabelled" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this suite is about two models disagreeing; it means nothing without both");
  process.exit(1);
}

const engine = await boot({ name: "mislabelled", seed: false, registry: local.registry });
const at = (path) => `${engine.url}${path}${path.includes("?") ? "&" : "?"}token=${engine.token}`;
const problems = [];

async function install(id) {
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 900; i += 1) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((candidate) => candidate.model === id);
    if (job?.state === "done") return;
    if (job?.state === "failed") throw new Error(`${id}: ${job.error ?? "install failed"}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`${id}: still installing after 450s`);
}

for (const id of MODELS) {
  console.log(`installing ${id}…`);
  await install(id);
}

if ((await (await fetch(at("/onboarding"))).json()).recognition !== true) {
  console.log("this daemon has no recogniser: nothing to drive");
  engine.stop();
  process.exit(0);
}

// The live model is pinned to the bad one on purpose; the specialist is left to `pick_pair`, which
// is the path a user takes by naming two languages and touching nothing else.
await fetch(at("/settings/models"), {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ role: "live", model: LIVE }),
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
// Vietnamese and English declared, set the way the app sets it — before the page that reads it
// exists. Driving the session over a socket of our own would send no audio: the frames come from
// the browser's own capture worklet, which is the path being tested.
await context.addInitScript(
  ([key, value]) => window.localStorage.setItem(key, value),
  ["summo.capture", JSON.stringify({ lanes: ["mic"], spoken: ["vi", "en"] })],
);
const page = await context.newPage();
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
  waitUntil: "domcontentloaded",
});
await page.locator("header").waitFor({ timeout: 20_000 });
const later = page.getByRole("button", { name: "Để sau" });
if ((await later.count()) > 0) await later.click();
await page
  .getByRole("button", { name: /Bắt đầu ghi/ })
  .first()
  .click();

for (let i = 0; i < 90; i += 1) {
  if (/refined an utterance/.test(plain(engine))) break;
  await new Promise((resolve) => setTimeout(resolve, 1000));
}

const log = plain(engine);
await page
  .getByRole("button", { name: /Dừng|Kết thúc/ })
  .first()
  .click()
  .catch(() => undefined);
await new Promise((resolve) => setTimeout(resolve, 1500));

const said = [...log.matchAll(/(warm slot|refining with|refined an utterance|refine skipped).*/g)]
  .map((m) => m[0].trim())
  .slice(0, 12);
for (const line of said) console.log(`  ${line}`);

// The assertion. `model=` on the refine line names which one produced the revision, and it must be
// the specialist — not the multilingual model that mislabelled the sentence in the first place.
const refined = [...log.matchAll(/refined an utterance.*model=(\S+)/g)].map((m) => m[1]);
if (refined.length === 0) {
  problems.push(
    "the specialist never refined a line: the declared languages did not reach `Refiner`",
  );
} else if (!refined.some((id) => id.includes("gipformer"))) {
  problems.push(`refined, but not by the specialist: ${refined.join(", ")}`);
} else {
  console.log(`the specialist rescued ${refined.length} line(s)`);
}

await browser.close();
engine.stop();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\nmislabelled ok");
