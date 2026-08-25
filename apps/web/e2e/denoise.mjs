/**
 * Noise suppression, from a card on a screen to the audio a decoder hears.
 *
 * `Task::Denoise` was in the manifest enum from the first commit and named nothing. No model in the
 * registry, no runtime, no stage in the pipeline; the only code that ever matched on it turned it
 * into the words "noise suppression" for a label with nothing behind them. Four other settings in
 * this app have been in that state — `models.refine`, `models.vad`, `models.speaker`,
 * `interface.theme` — and every one of them was found by a user rather than by a suite, because
 * "written to the settings file" and "reaches a recording" look identical from outside.
 *
 * So this drives the whole chain and skips none of it:
 *
 * - the enhancer appears on the models screen as its own section and installs,
 * - **the card's button is what turns it on** — not a `fetch` this file makes, because the button
 *   is the half that was missing every previous time,
 * - a recording started afterwards reports it on `/status`, which is the only outside evidence the
 *   session was built with one,
 * - words still arrive: an enhancer that quietly broke recognition would otherwise look like a
 *   success here,
 * - and the card can turn it off again, which no other role needs and this one cannot do without.
 *
 * The audio is the ordinary clean Vietnamese fixture. Proving that GTCRN *improves* a noisy
 * recording is a benchmark, not a suite — it needs a noisy corpus and a word error rate, and it
 * belongs beside the other measurements rather than in a Playwright run. What this proves is that
 * the model runs where it was wired to run.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-fleurs.wav");
const problems = [];

const MODELS = ["gipformer-65m", "silero-vad-v5", "gtcrn-16k"];
const local = await mirror(MODELS, { name: "denoise" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this suite is about an enhancer running inside a recording; it needs all three");
  process.exit(1);
}

const engine = await boot(process.argv, { name: "denoise", registry: local.registry });
const { url: appUrl, port, token } = engine;
const at = (path) => `${appUrl}${path}${path.includes("?") ? "&" : "?"}token=${token}`;
const status = async () => (await fetch(at("/status"))).json();
const chosen = async () => (await (await fetch(at("/catalogue"))).json()).chosen;

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

await fetch(at("/settings/models"), {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ role: "live", model: "gipformer-65m" }),
});

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
  viewport: { width: 1280, height: 900 },
  colorScheme: "dark",
});
const page = await context.newPage();
page.on("pageerror", (error) => problems.push(`pageerror: ${error.message}`));

try {
  // ---- the card, and its button ----------------------------------------------
  //
  // Clicked rather than posted. `roleFor` returned `null` for every task but speech and
  // translation, so a denoise card rendered with no control on it at all — installed, listed,
  // unreachable. A `fetch` here would have passed over exactly that.
  await page.goto(`${appUrl}?port=${port}&token=${token}#/models`, { waitUntil: "networkidle" });
  await page.getByTestId("models").waitFor({ timeout: 20000 });

  const card = page.locator("article", { hasText: "gtcrn-16k" }).first();
  if ((await card.count()) === 0) {
    problems.push("the enhancer has a manifest and no card — the screen dropped its whole section");
  } else {
    const use = card.getByRole("button", { name: "Dùng", exact: true });
    if ((await use.count()) === 0) {
      problems.push("the enhancer's card has no way to turn it on");
    } else {
      await use.click();

      let picked = null;
      for (let i = 0; i < 40 && picked !== "gtcrn-16k"; i++) {
        await page.waitForTimeout(250);
        picked = (await chosen())?.denoise ?? null;
      }
      if (picked !== "gtcrn-16k") {
        problems.push(`the button did not reach the settings: ${JSON.stringify(picked)}`);
      } else {
        console.log("chosen from the card");
      }
    }
  }

  // ---- and it reaches a recording -------------------------------------------
  await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });
  await page
    .getByTestId("home")
    .getByRole("button", { name: /Bắt đầu ghi/ })
    .click();

  const firstLine = page.locator('[data-testid="transcript-line"]').first();
  await firstLine.waitFor({ timeout: 120000 }).catch((error) => {
    // The failure this catches is the one worth catching: an enhancer that runs and destroys the
    // audio produces a recording with no words in it, and every other assertion here would still
    // pass. `finalize` denoises before it decodes, so no line at all means the clean-up ate them.
    console.log("--- daemon log ---\n" + engine.log().slice(-4000));
    throw error;
  });

  const during = await status();
  console.log(`live ${during.live_model}, enhancer ${during.denoise_model}, ${during.state}`);
  if (during.denoise_model !== "gtcrn-16k") {
    problems.push(
      `the enhancer never reached the session: ${JSON.stringify(during.denoise_model)}`,
    );
  }
  if (engine.log().includes("the speech enhancer failed")) {
    problems.push("the enhancer loaded and then failed on every utterance");
  }

  const said = await firstLine.innerText();
  console.log(`heard through the enhancer: ${JSON.stringify(said.slice(0, 60))}`);

  await page.waitForTimeout(6000);
  await page
    .getByRole("button", { name: /Dừng ghi/ })
    .first()
    .click();
  await page.waitForTimeout(2000);

  // ---- and it can be turned off again ----------------------------------------
  //
  // The only role that needs this. Every other one falls back to something sensible when unset, so
  // changing your mind means choosing a different model; an enhancer that is unset is *off*, and
  // without a control the first click would be permanent short of editing the settings file.
  await page.goto(`${appUrl}?port=${port}&token=${token}#/models`, { waitUntil: "networkidle" });
  await page.getByTestId("models").waitFor({ timeout: 20000 });

  const again = page.locator("article", { hasText: "gtcrn-16k" }).first();
  const off = again.getByRole("button", { name: "Tắt", exact: true });
  if ((await off.count()) === 0) {
    problems.push("a chosen enhancer cannot be turned off from its card");
  } else {
    await off.click();
    let still = "gtcrn-16k";
    for (let i = 0; i < 40 && still; i++) {
      await page.waitForTimeout(250);
      still = (await chosen())?.denoise ?? null;
    }
    if (still) problems.push(`turning it off did not stick: ${JSON.stringify(still)}`);
    else console.log("turned off again");
  }

  await page.screenshot({ path: "/tmp/shots/denoise.png" });
} finally {
  await browser.close();
  await engine.stop();
}

console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\ndenoise ok");
if (problems.length) process.exit(1);
