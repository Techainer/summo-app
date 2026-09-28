/**
 * The whole arrangement, on real mixed-language audio, against a known ground truth.
 *
 * Everything else that covers this is either a unit test or drives one language. `bilingual.mjs`
 * comes closest and pairs `whisper-base` with a specialist — a good combination. This drives the
 * combination people actually have and that produced the report: **`whisper-tiny` listening**, two
 * languages declared, a specialist for one of them and nothing for the other.
 *
 * `fixtures/bilingual.wav` is four FLEURS clips — Vietnamese, English, Vietnamese, English — with
 * silence between them so the detector closes each one. `fixtures/README.md` records what was
 * said, so this suite knows what the right answer is.
 *
 * **It does not assert the words.** Which words a model returns is allowed to change, and a suite
 * that pins them turns every model update into a failure. What it asserts is everything the last
 * three releases were about, each of which was a real defect in a real meeting:
 *
 * 1. no line is in a language nobody declared — a Vietnamese sentence came back labelled `ru`,
 * 2. both declared languages are present — the Vietnamese half was being lost entirely,
 * 3. the Vietnamese line came from the specialist, not from the model that mislabelled it,
 * 4. no line is a repetition loop — `今天今天今天…` twenty-four times reached a transcript,
 * 5. the declared language with no specialist has the general model pinned to it.
 *
 * **What it is and is not.** It is the only place the real combination is driven on real mixed
 * audio, and it would catch a systematic failure — every line mislabelled, one language gone, a
 * decoder never loaded, a transcript full of loops. It is not a deterministic guard for any one of
 * those bugs: which language `whisper-tiny` guesses on a given clip varies between runs, so a run
 * that happens to guess right passes whatever is broken underneath. The deterministic guards are
 * the unit tests in `segment.rs`, `refine.rs` and `hallucination.rs`; this is what proves they add
 * up to a working meeting.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { boot, plain } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/bilingual.wav");

/** The languages the recording is in, and what the app is told. */
const DECLARED = ["vi", "en"];
/** Deliberately the worst multilingual model: it is the one that produced the report. */
const LIVE = "whisper-tiny";
const MODELS = [LIVE, "gipformer-1.5-68m", "silero-vad-v5"];

const local = await mirror(MODELS, { name: "multilingual" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this suite is about two languages and two models; it needs both");
  process.exit(1);
}

const engine = await boot({ name: "multilingual", seed: false, registry: local.registry });
const at = (p) => `${engine.url}${p}${p.includes("?") ? "&" : "?"}token=${engine.token}`;
const problems = [];

async function install(id) {
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 900; i += 1) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((c) => c.model === id);
    if (job?.state === "done") return;
    if (job?.state === "failed") throw new Error(`${id}: ${job.error ?? "install failed"}`);
    await new Promise((r) => setTimeout(r, 400));
  }
  throw new Error(`${id}: still installing after 360s`);
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

// Pinned to the bad model on purpose. The specialist is left to `pick_pair`, which is the path a
// user takes by naming two languages and touching nothing else.
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
await context.addInitScript(
  ([k, v]) => window.localStorage.setItem(k, v),
  ["summo.capture", JSON.stringify({ lanes: ["mic"], spoken: DECLARED })],
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

// The fixture is 22.4 s. The extra is the last utterance settling and its second pass landing.
await new Promise((r) => setTimeout(r, 34_000));
await page
  .getByRole("button", { name: /Dừng|Kết thúc/ })
  .first()
  .click()
  .catch(() => undefined);
await new Promise((r) => setTimeout(r, 3000));

const log = plain(engine);

// ---- 5. every declared language has a decoder built for it ------------------
const refining = log.match(/refining with models=(\[.*\])/);
console.log(`listening with ${LIVE}, refining with ${refining?.[1] ?? "nothing"}`);
if (!refining) {
  problems.push("the daemon never said what it is refining with");
} else {
  if (!refining[1].includes("gipformer")) {
    problems.push(`Vietnamese has a specialist and it was not loaded: ${refining[1]}`);
  }
  // Nothing in the registry specialises in English, so it must be the general model pinned to it.
  if (!/whisper[^",]*\[en\]/.test(refining[1])) {
    problems.push(`English was declared and no decoder was pinned to it: ${refining[1]}`);
  }
}

// ---- the transcript on disk, which is the copy that outlives the tab --------
const library = await (await fetch(at("/library"))).json();
const id = library.groups?.[0]?.meetings?.[0]?.id;
const detail = id ? await (await fetch(at(`/meetings/${id}`))).json() : null;
const lines = detail?.transcript ?? [];

console.log("");
for (const line of lines) console.log(`  [${line.language ?? "?"}] ${line.text}`);
console.log("");

if (lines.length < 2) {
  problems.push(`expected the fixture's utterances, got ${lines.length}`);
}

const base = (code) => String(code).toLowerCase().split(/[-_]/)[0];

// ---- 1. no line is in a language nobody declared ---------------------------
//
// `whisper-tiny` answered `zh`, `he`, `ja` and `ru` on this recording's languages. A label that
// survives to the file decides which way the line is translated for as long as the file exists.
for (const line of lines) {
  if (line.language && !DECLARED.includes(base(line.language))) {
    problems.push(
      `a line is filed under a language nobody declared: ${line.language} — ${line.text}`,
    );
  }
}

// ---- 2. both halves of the meeting are there -------------------------------
const heard = new Set(lines.filter((l) => l.language).map((l) => base(l.language)));
if (!heard.has("vi") || !heard.has("en")) {
  problems.push(`a recording with both languages in it produced ${JSON.stringify([...heard])}`);
} else {
  console.log(`both languages reached the file: ${[...heard].join(", ")}`);
}

// ---- 3. the Vietnamese came from the model that can hear it ----------------
const rescued = [...log.matchAll(/refined an utterance.*model=(\S+)/g)].map((m) => m[1]);
if (!rescued.some((m) => m.includes("gipformer"))) {
  problems.push(
    `no Vietnamese line was rescued by the specialist: refined by ${rescued.join(", ") || "nothing"}`,
  );
} else {
  console.log(`the specialist rescued ${rescued.length} line(s)`);
}

// ---- 4. nothing is a repetition loop ---------------------------------------
//
// `大家看今天今天今天…` twenty-four times reached a real transcript, and then acquired subtitles.
// Counted per character for scripts without word spaces, which is what hid it.
for (const line of lines) {
  const tokens = /[぀-ヿ㐀-鿿]/.test(line.text)
    ? [...line.text.replace(/\s+/g, "")]
    : line.text.toLowerCase().split(/\W+/).filter(Boolean);
  let run = 1;
  for (let i = 1; i < tokens.length; i += 1) {
    run = tokens[i] === tokens[i - 1] ? run + 1 : 1;
    if (run > 4) {
      problems.push(`a line repeats itself: ${line.text.slice(0, 60)}`);
      break;
    }
  }
}

await browser.close();
engine.stop();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\nmultilingual ok");
