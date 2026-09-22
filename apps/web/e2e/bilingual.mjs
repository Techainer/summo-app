/**
 * Two speech models and two subtitle languages on one meeting that really has two languages in it.
 *
 * `refine_model` was a setting that did nothing. The models screen has offered "use for refine"
 * since it had buttons, `/settings/models` accepted the role and wrote it to disk, and no session
 * ever read it — `HybridSession` was written, tested, exported and never constructed. So this suite
 * exists to prove the wiring end to end rather than in unit tests that would have passed the whole
 * time it was disconnected.
 *
 * ## Why the fixture changed
 *
 * It used to drive Vietnamese audio, and said so: *"provoking an English sentence out of a
 * Vietnamese fixture is not something a suite should try to arrange"*. So the routing decision —
 * the entire point of pairing two models — was left to unit tests, and the two halves of the
 * feature that only exist together were never run together. Both bugs below lived in that gap.
 *
 * `fixtures/bilingual.wav` is two Vietnamese sentences and two English ones, alternating. That is
 * the meeting the feature was built for: a Vietnamese standup with an English customer on the call.
 *
 * What it drives, with real models and real audio:
 *
 * - a session started with nothing named picks up **both** models from the settings file,
 * - the daemon reports the pair on `/status`, which is what an interface can see,
 * - Vietnamese speech decoded live by Whisper is **revised** by Gipformer, in place, without the
 *   recording stopping — the revision is the whole feature and the only proof the second decoder
 *   ran at all,
 * - the English sentences in the same recording are **left alone** by the Vietnamese-only model,
 * - two subtitle languages on a bilingual meeting render **each line into the other one**,
 * - two subtitle languages on a *monolingual* line render **both**, one under the other,
 * - the same model in both roles does not refuse the recording.
 */
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/bilingual.wav");
const problems = [];
/** The exact text of a line the second model rewrote, so the vault can be held to it later.
 *
 * Named apart from the local `refined` inside the block that finds it — that one is a boolean, and
 * a `let` of the same name shadowed this one, so the assertion at the bottom read `null` and said
 * nothing at all. A silent assertion is worse than a missing one. */
let refinedLine = null;

// Whisper hears ninety-nine languages badly and reports which one it heard; Gipformer hears
// Vietnamese and nothing else, accurately. That asymmetry is the entire reason for the feature.
// `whisper-base` rather than `whisper-tiny` for the live role: this fixture is only worth driving
// if the language of each sentence is identified, and tiny is measurably worse at that. `tiny` is
// still installed — it is the second `langs: ["*"]` manifest, and the last block needs one.
// SMALL100 is 610 MB and is what makes the subtitle assertions mean anything.
const MODELS = ["whisper-tiny", "whisper-base", "gipformer-1.5-68m", "silero-vad-v5", "small100"];
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

// Asked rather than written, for the reason spelled out in `in-meeting.mjs`: a suite that POSTs
// `llm.translator` tests a setup no user performs, and the daemon used to resolve an unconfigured
// translator to an `ollama` endpoint nobody was running and report success.
{
  const plan = await (await fetch(at("/settings/plan"))).json();
  if (plan.translation.using !== "small100") {
    console.error(
      "installing SMALL100 is not enough to translate with it: " + JSON.stringify(plan.translation),
    );
    process.exit(1);
  }
}

const pick = (role, model) =>
  fetch(at("/settings/models"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ role, model }),
  });

// ---- the app suggests the pairing, before anybody knows to ask for it ------
//
// Nothing has ever suggested a second model. `Recommendation::pair` exists, only the CLI calls it,
// and it would not answer this: it ranks the second model by accuracy *on the chosen language*, so
// for a Vietnamese meeting it compares Whisper against Gipformer on Vietnamese, finds Whisper far
// worse, and recommends nothing — for exactly the meeting where the second model matters most. The
// English sentences are the point, and accuracy on Vietnamese says nothing about them.
//
// This is the headline case: a Vietnamese company with an English customer on the call. Gipformer
// declares `vi` and returns Vietnamese-shaped noise for everything else.
{
  await pick("live", "gipformer-1.5-68m");
  const plan = await (await fetch(at("/settings/plan"))).json();
  const suggested = plan.second_pass?.suggested;
  if (suggested?.id !== "whisper-base" && suggested?.id !== "whisper-tiny") {
    problems.push(
      `a Vietnamese-only model was left to answer for English: ${JSON.stringify(suggested)}`,
    );
  } else {
    console.log(`suggested second pass: ${suggested.id} — ${suggested.reason}`);
  }

  // And once one is chosen, the suggestion stops. Advice beside a decision somebody already made
  // is an argument rather than advice.
  await pick("refine", "whisper-base");
  const settled = await (await fetch(at("/settings/plan"))).json();
  if (settled.second_pass?.suggested) {
    problems.push("the app kept recommending a second model after one was chosen");
  }
  if (settled.second_pass?.model !== "whisper-base") {
    problems.push(
      `the chosen second model is not on the plan: ${JSON.stringify(settled.second_pass)}`,
    );
  }
  await pick("refine", "");
}

// Exactly what pressing the two buttons on the models screen writes. Nothing here reaches into the
// session — the point is that the *settings file* is enough, which is the half that was missing.
await pick("live", "whisper-base");

// ---- the same model twice must not break recording -------------------------
//
// Reachable by pressing "use" and then "use for refine" on one row, and `SessionSpec::validate`
// refuses the pair outright — so without the guard this is a record button that fails.
{
  await pick("refine", "whisper-base");
  // `/catalogue` is what the models screen reads to draw which row is chosen for what, so this is
  // the same answer a user would see rather than a private corner of the settings file.
  const { chosen } = await (await fetch(at("/catalogue"))).json();
  if (chosen?.refine !== "whisper-base") {
    problems.push(`the models screen would not show the second role: ${JSON.stringify(chosen)}`);
  }
}

// And the pair the rest of this file is about, written last so the block above cannot decide it.
await pick("refine", "gipformer-1.5-68m");

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
await firstLine.waitFor({ timeout: 120000 }).catch(async (error) => {
  // What the daemon thinks is happening, not just what it has printed. A session that never
  // started and a session recording silence look identical in the log — it says nothing either
  // way — and they are opposite bugs: one is the record button, the other is the audio reaching
  // it. Twice this failed here and the log could not tell them apart.
  console.log("--- /status ---\n" + JSON.stringify(await status().catch((e) => String(e))));
  console.log("--- daemon log ---\n" + engine.log().slice(-4000));
  throw error;
});

/** Every line on screen: what it was heard as, and the languages of the subtitles under it. */
const lines = () =>
  page.$$eval('[data-testid="transcript-line"]', (nodes) =>
    nodes.map((node) => ({
      seq: node.getAttribute("data-seq"),
      spoken: node.getAttribute("lang"),
      text: (node.textContent ?? "").trim(),
      subtitles: [
        ...(node.parentElement?.querySelectorAll('[data-testid="transcript-translation"]') ?? []),
      ].map((sub) => sub.getAttribute("lang")),
    })),
  );

/** Base tag, so `en-US` from a runtime compares against `en` from a manifest. */
const base = (code) => (code ?? "").toLowerCase().split(/[-_]/)[0];

// ---- the daemon is running both, and says so -------------------------------
{
  const now = await status();
  console.log(`live ${now.live_model}, refine ${now.refine_model}, ${now.state}`);
  if (now.live_model !== "whisper-base") {
    problems.push(`the live model came from nowhere useful: ${now.live_model}`);
  }
  if (now.refine_model !== "gipformer-1.5-68m") {
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
      refinedLine = line.trim();
      console.log(`refined and on screen: ${JSON.stringify(refinedLine.slice(0, 60))}`);
    }
  }

  const during = await status();
  if (during.state !== "recording") {
    problems.push("the meeting ended while the second model was working");
  }
  await page.screenshot({ path: "/tmp/shots/bilingual.png" });
}

// ---- and the English half of the same recording is left alone --------------
//
// The other half of the decision, and the half no suite could reach while the fixture was
// monolingual. Running Gipformer on an English sentence does not fail — it returns Vietnamese-
// shaped noise over a line that was correct, which is worse than the text it replaced. So the
// routing declines it, and says so once per language.
{
  let declined = false;
  for (let i = 0; i < 120 && !declined; i++) {
    await page.waitForTimeout(500);
    declined = engine.log().includes("claims no such language");
  }
  if (!declined) {
    console.log("--- daemon log ---\n" + engine.log().slice(-3000));
    problems.push(
      "a Vietnamese-only second model was never asked to decline English — either no English " +
        "was heard in a recording that is half English, or it was refined anyway",
    );
  } else {
    console.log("the Vietnamese-only model declined the English lines");
  }
}

// ---- two languages means each line into the other one ----------------------
//
// The rule arrived on the offline path and the live path never read it: `Pending` carried a
// sequence number and a string, so every line went to every target. On this meeting with `vi` and
// `en` both chosen, that put a Vietnamese "translation" of each Vietnamese line underneath it and
// paid a request for the privilege — while the note under the control said, correctly, that each
// line is rendered into the other one.
//
// Checked per line against the language that line was heard in, because the total is the same
// either way and the count of subtitles is not the rule — a line in a third language legitimately
// gets both.
{
  await page.getByLabel("Dịch trực tiếp").selectOption("vi");
  const refused = await page
    .locator("text=/local-mt|in-process/")
    .first()
    .waitFor({ timeout: 4000 })
    .then(() => true)
    .catch(() => false);
  if (refused) {
    console.log("SKIPPED translation: this binary has no in-process translator (needs mt-onnx).");
    console.log("  The release build has it — see FEATURES in scripts/bundle.sh.");
    await browser.close();
    await engine.stop();
    console.log(
      problems.length
        ? `\nPROBLEMS:\n  ${problems.join("\n  ")}`
        : "\nbilingual ok (translation skipped)",
    );
    process.exit(problems.length ? 1 : 0);
  }
  await page.getByLabel("Thêm một ngôn ngữ nữa").selectOption("en");

  let both = null;
  for (let i = 0; i < 240; i++) {
    await page.waitForTimeout(500);
    const now = await status();
    const into = now.translate_into ?? [];
    if (into.includes("vi") && into.includes("en")) {
      both = now;
      break;
    }
  }
  if (!both) {
    problems.push("the two subtitle languages never reached the running session");
  } else {
    console.log(`translate: → ${both.translate_into}, still ${both.state}`);
    if (both.state !== "recording") {
      problems.push("the meeting ended when two subtitle languages were chosen");
    }

    // Wait for subtitles in *both* directions. A meeting where only one of them ever appears is
    // either a fixture whose second language was never recognised or a pass that is not running,
    // and both are worth failing over.
    let seen = [];
    for (let i = 0; i < 240; i++) {
      await page.waitForTimeout(500);
      seen = await lines();
      const langs = new Set(seen.flatMap((line) => line.subtitles));
      if (langs.has("vi") && langs.has("en")) break;
    }

    const langs = new Set(seen.flatMap((line) => line.subtitles));
    if (!(langs.has("vi") && langs.has("en"))) {
      console.log("--- daemon log ---\n" + engine.log().slice(-3000));
      problems.push(
        `a meeting with two languages in it produced subtitles in ${JSON.stringify([...langs])}`,
      );
    } else {
      console.log(`subtitles arrived in both directions: ${[...langs].join(" ↔ ")}`);
    }

    // The bug itself: a line carrying a subtitle in the language it was spoken in.
    //
    // Asserted against the line's own `lang` rather than against how many subtitles it has. The
    // count is not the rule, and this suite learned that the hard way on CI: Whisper heard noise on
    // that runner as Chinese, so `和等进` was correctly given *both* a Vietnamese and an English
    // subtitle — it is in neither — and a check that read "more than one subtitle" called the
    // correct answer a bug.
    //
    // Lines the recogniser did not label are skipped rather than trusted: with no language there is
    // nothing to compare, and the daemon translates them into everything on purpose.
    const wrong = seen.filter(
      (line) => line.spoken && line.subtitles.some((sub) => base(sub) === base(line.spoken)),
    );
    if (wrong.length > 0) {
      problems.push(
        `${wrong.length} line(s) were translated into the language they were already in: ` +
          JSON.stringify(wrong.slice(0, 2)),
      );
    }

    // And the rule did something: at least one line was owed a subtitle it did not get, which is
    // what "each into the other one" means and what a daemon translating everything into everything
    // would never produce.
    const held = seen.filter(
      (line) =>
        ["vi", "en"].includes(base(line.spoken)) &&
        line.subtitles.length === 1 &&
        base(line.subtitles[0]) !== base(line.spoken),
    );
    if (held.length === 0) {
      problems.push(
        "no line was rendered into the other language only — either nothing was recognised as " +
          `Vietnamese or English, or both passes ran on everything: ${JSON.stringify(seen.slice(0, 3))}`,
      );
    } else {
      console.log(`${held.length} line(s) rendered into the other language and no further`);
    }
    await page.screenshot({ path: "/tmp/shots/bilingual-two-way.png" });
  }
}

// ---- two readers, on the other hand, both get a subtitle -------------------
//
// The neighbouring mistake, and the reason `translations` is a list. The daemon has sent one
// translation event per target since targets became a list; the reducer kept one of them, so the
// second subtitle overwrote the first and which one you saw depended on the order two requests
// happened to come home in. The control that offers a second language exists because "a meeting can
// have more than one reader" — and that reader never saw a line.
//
// Japanese and English on a Vietnamese line: both apply, so both must be on screen at once.
{
  await page.getByLabel("Dịch trực tiếp").selectOption("en");
  await page.getByLabel("Thêm một ngôn ngữ nữa").selectOption("ja");

  let paired = null;
  for (let i = 0; i < 300; i++) {
    await page.waitForTimeout(500);
    const seen = await lines();
    paired = seen.find((line) => line.subtitles.includes("en") && line.subtitles.includes("ja"));
    if (paired) break;
  }

  if (!paired) {
    console.log("--- daemon log ---\n" + engine.log().slice(-3000));
    problems.push("two readers asked for subtitles and no line ever carried both");
  } else {
    console.log(`one line, two readers: ${JSON.stringify(paired.text.slice(0, 40))}`);
  }
  await page.screenshot({ path: "/tmp/shots/bilingual-two-readers.png" });
}

// ---- a multilingual second model is not "a model for no language" ----------
//
// The routing compared the live model's reported language against the refine model's `langs` with
// `contains`, which reads every entry as a literal code. `whisper-tiny` and `whisper-base` publish
// `langs: ["*"]` — a star equals nothing — so pairing either as the second opinion refined *no
// utterance at all*, and said so only at `debug`. The setting applied, `/status` named the model,
// and the transcript was never revised.
//
// Asserted on the absence of the skip notice rather than the presence of a revision: a second model
// that runs and *agrees* produces no `Event::Revise` (`HybridSession::refine` returns `None` on
// identical text), so requiring a revision here would be a coin flip. "Was this pairing asked to do
// anything" is the question, and the skip notice is the only honest answer to it.
{
  await page.getByLabel("Mô hình phụ").selectOption("whisper-tiny");

  let swapped = false;
  for (let i = 0; i < 240 && !swapped; i++) {
    await page.waitForTimeout(500);
    swapped = (await status()).refine_model === "whisper-tiny";
  }

  if (!swapped) {
    console.log("--- daemon log ---\n" + engine.log().slice(-3000));
    problems.push("the multilingual second model never reached the running session");
  } else {
    // From here, not from the start of the run: Gipformer legitimately declines the English half of
    // this fixture, and the block above asserts that it does. Reading the whole log would blame the
    // wrong model for the right behaviour.
    const mark = engine.log().length;
    await page.waitForTimeout(20000);
    const since = engine.log().slice(mark);
    if (since.includes("claims no such language")) {
      problems.push('a `langs: ["*"]` model was refused every line — the star read as a language');
    } else {
      console.log("multilingual second model accepted the spoken language");
    }
    const still = await status();
    if (still.state !== "recording") {
      problems.push("the meeting ended when the multilingual second model was chosen");
    }
  }
}

await page
  .getByRole("button", { name: /Dừng ghi/ })
  .first()
  .click();
await page.waitForTimeout(3000);

// What the screen showed also has to be what is on disk.
//
// Everything above this reads the browser, and the browser was right about both of the things this
// block checks while the vault was wrong about both:
//
// - the refine pass produced `Event::Revise`, the transcript on screen changed, and the event loop
//   applied the *runner's* events to the document before the refiner had produced anything — so the
//   saved meeting kept the first model's text and the entire second-model feature changed nothing
//   that outlives the tab;
// - a subtitle was an event on a socket and a node in a React tree. Press stop and it was gone,
//   with no way to get it back except paying for the whole meeting again.
//
// Read from the vault rather than from an endpoint, because the promise is a folder of files.
{
  const meetings = join(engine.home, "vault", "meetings");
  const files = readdirSync(meetings).filter((name) => name.endsWith(".md"));
  const body = files.map((name) => readFileSync(join(meetings, name), "utf8")).join("\n");

  if (!/lang:\s*vi/i.test(body) || !/lang:\s*en/i.test(body)) {
    problems.push("the saved meeting does not record which language each line was spoken in");
  }

  // The second model's text, matched against what the screen showed rather than against a marker:
  // the saved format records `seq`, `end` and `lang` and deliberately does not record which model
  // won, so the only honest question is whether the words in the file are the words on screen.
  //
  // A prefix, because a revision can land again between the screenshot and the stop.
  if (refinedLine) {
    const head = refinedLine.slice(0, 40);
    if (!body.includes(head)) {
      console.log(body.slice(0, 1500));
      problems.push(
        `the second model revised the transcript on screen and not in the vault: ${JSON.stringify(head)}`,
      );
    } else {
      console.log("the refined text reached the file, not just the screen");
    }
  }

  const translations = join(engine.home, "vault", "translations");
  const subtitles = existsSync(translations) ? readdirSync(translations) : [];
  if (subtitles.length === 0) {
    problems.push("a meeting was subtitled in three languages and the vault has no record of any");
  } else {
    console.log(`subtitles kept: ${subtitles.join(", ")}`);
  }
}

await browser.close();
await engine.stop();

console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nbilingual ok");
if (problems.length) process.exit(1);
