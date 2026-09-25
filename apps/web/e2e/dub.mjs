/**
 * A meeting, spoken in another language, over its own recording — from the app.
 *
 * The pipeline has worked for releases. `summo dub` had tests, two voices were published for it,
 * and the README listed dubbing under what works today — while inside the app there was no route,
 * no screen and not one string in the catalogue. From where a user stands, that is a feature that
 * does not exist.
 *
 * This drives the door: install a voice, dub a translated meeting through the daemon, and watch the
 * result turn into a track the player can switch to. Each of those has failed separately before —
 * the `import` lane was served under a name no player asked for, so every imported meeting drew a
 * transport that answered `no such lane`.
 *
 * ## What it checks before it downloads anything
 *
 * The refusals, which are most of what a user meets. Dubbing a language nobody translated into, a
 * language no installed voice speaks, and a language that is really a path — each has to be an
 * answer to the request rather than a job id that dies a second later in a list nobody is reading.
 */
import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";

const problems = [];
const engine = await boot(process.argv, { name: "dub" });
const { url: appUrl, port, token, home } = engine;
const MEETING = "01E2E0";

const ask = async (path, body) => {
  const response = await fetch(`${engine.url}${path}?token=${token}`, {
    method: body ? "POST" : "GET",
    headers: body ? { "content-type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  let parsed = null;
  try {
    parsed = JSON.parse(text);
  } catch {
    // A refusal is sometimes plain text. The caller wants the words either way.
  }
  return { ok: response.ok, status: response.status, body: parsed, text };
};

try {
  // ---- refusals, before any download --------------------------------------
  //
  // Nothing has been translated and no voice is installed, which is the state every new user is in.
  const untranslated = await ask(`/meetings/${MEETING}/dub`, { lang: "en" });
  if (untranslated.ok) problems.push("dubbing an untranslated meeting was accepted");
  if (!/translation/i.test(untranslated.text)) {
    problems.push(`an untranslated meeting was refused without saying why: ${untranslated.text}`);
  }

  const nowhere = await ask("/meetings/nosuchmeeting/dub", { lang: "en" });
  if (nowhere.ok) problems.push("dubbing a meeting that does not exist was accepted");

  // The language reaches a file name. `..` must not survive the trip, and the refusal must be
  // about the language rather than about a path — a 500 here would mean it got further than it
  // should have.
  const traversal = await ask(`/meetings/${MEETING}/dub`, { lang: "../../etc/passwd" });
  if (traversal.ok) problems.push("a language that is a path was accepted");
  if (traversal.status >= 500) {
    problems.push(`a path-shaped language reached something that crashed: ${traversal.text}`);
  }

  // ---- a translation on disk ----------------------------------------------
  //
  // Written rather than produced by the translate route, which needs a translation model and a
  // 600 MB download to say something this suite does not test. The format is `summo_vault::
  // translation`'s own, and its parser is what reads this back.
  const lines = [
    [0, "00:12:04", "Let us talk about the budget"],
    [1, "00:13:10", "I think we should settle the spec first"],
    [2, "00:14:02", "Then let us settle it today"],
    [3, "00:15:30", "I will send a draft this afternoon"],
  ];
  const translations = join(home, "vault/translations");
  mkdirSync(translations, { recursive: true });
  writeFileSync(
    join(translations, `${MEETING}.en.md`),
    `<!-- summo:translation lang:en -->\n\n` +
      lines.map(([seq, at, text]) => `[${at}] ${text} <!-- seq:${seq} -->\n`).join(""),
  );

  const noVoice = await ask(`/meetings/${MEETING}/dub`, { lang: "en" });
  if (noVoice.ok) problems.push("dubbing with no voice installed was accepted");
  // The refusal has to name the way out. "No voice" and "no voice, and here is what to pull" are
  // different sentences to somebody who has never seen the models screen.
  if (!/voice/i.test(noVoice.text)) {
    problems.push(`a missing voice was refused without naming one: ${noVoice.text}`);
  }
  console.log("refusals ok: no translation, no meeting, no voice, no path");

  // ---- the voice -----------------------------------------------------------
  const VOICE = "vits-en-ljspeech";
  const install = await ask("/installs", { id: VOICE });
  if (!install.ok) {
    throw new Error(`the voice could not be installed: ${install.text}`);
  }
  // Polled by *model id*, which is how `/installs` is keyed — asking for the same model twice is
  // somebody clicking a button twice, and the right answer is the job already running rather than
  // a second one fighting it for the same staging file. There is no separate job id to hold on to.
  const installed = await until(
    async () => {
      const job = await ask(`/installs/${encodeURIComponent(VOICE)}`);
      if (!job.ok) throw new Error(`the install job vanished: ${job.text}`);
      return job.body?.state === "done" || job.body?.state === "failed" ? job.body : null;
    },
    300_000,
    "the voice never finished installing",
  );
  if (installed.state !== "done") {
    throw new Error(`the voice failed to install: ${JSON.stringify(installed)}`);
  }
  console.log("voice installed");

  // ---- the dub -------------------------------------------------------------
  const started = await ask(`/meetings/${MEETING}/dub`, { lang: "en" });
  if (!started.ok) throw new Error(`the dub was refused: ${started.text}`);
  if (started.body?.meeting !== MEETING) {
    problems.push(`the job names the wrong meeting: ${started.text}`);
  }

  // Two passes over the lines, and the bar has to be able to say which. A job that reports no
  // progress at all is one a user watches with nothing to look at for minutes.
  const seen = new Set();
  const done = await until(
    async () => {
      const job = await ask(`/dubs/${started.body.id}`);
      if (job.body?.state === "speaking") seen.add(job.body.pass);
      if (job.body?.state === "done" || job.body?.state === "failed") return job.body;
      return null;
    },
    300_000,
    "the dub never finished",
  );
  if (done.state !== "done") throw new Error(`the dub failed: ${JSON.stringify(done)}`);
  if (done.lines !== lines.length) {
    problems.push(`dubbed ${done.lines} line(s) of ${lines.length}`);
  }
  if (!(done.duration_s > 0)) problems.push(`the dub is ${done.duration_s}s long`);
  console.log(
    `dubbed: ${done.lines}/${done.of} lines, ${done.duration_s.toFixed(1)}s, ` +
      `${done.natural} natural / ${done.adjusted} adjusted / ${done.overflowing} overflowing`,
  );

  // ---- and it is a track ---------------------------------------------------
  //
  // The part that has failed silently before. A file written under a name the player does not ask
  // for is a file nobody can hear.
  // Inside the run, not after it: `engine.stop()` takes the temporary home with it, so a check
  // down there would be asking about a directory that has been deleted — and would have "passed"
  // or "failed" for a reason that has nothing to do with dubbing.
  const dir = join(home, `audio/${MEETING}`);
  if (!existsSync(dir)) {
    problems.push(`the dub created no audio directory at ${dir}`);
  }
  const audio = existsSync(dir) ? readdirSync(dir) : [];
  if (!audio.includes("dub-en.wav")) {
    problems.push(`the dub is not where the player looks: ${audio.join(", ")}`);
  }

  const detail = await ask(`/meetings/${MEETING}`);
  if (!(detail.body?.dubs ?? []).includes("en")) {
    problems.push(`the meeting does not list its dub: ${JSON.stringify(detail.body?.dubs)}`);
  }

  // Served, with the right type, and seekable. A wav announced as Ogg is the kind of wrong that
  // works in one browser and not the next.
  const track = await fetch(`${engine.url}/meetings/${MEETING}/audio/dub-en?token=${token}`, {
    headers: { range: "bytes=0-1023" },
  });
  if (track.status !== 206) problems.push(`the dub track is not seekable: HTTP ${track.status}`);
  if (track.headers.get("content-type") !== "audio/wav") {
    problems.push(`the dub is served as ${track.headers.get("content-type")}`);
  }
  const head = Buffer.from(await track.arrayBuffer());
  if (head.subarray(0, 4).toString() !== "RIFF") {
    problems.push("what came back is not a wav");
  }

  // A lane nobody has dubbed is missing rather than unknown — the distinction lets the screen say
  // "not dubbed yet" instead of "unknown lane".
  const missing = await fetch(`${engine.url}/meetings/${MEETING}/audio/dub-ja?token=${token}`);
  if (missing.status !== 404) problems.push(`an undubbed language answered ${missing.status}`);

  // ---- and the player offers it -------------------------------------------
  const browser = await chromium.launch();
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 900 },
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && problems.push(`console: ${m.text()}`));

  await page.goto(`${appUrl}?port=${port}&token=${token}#/pages/${MEETING}`, {
    waitUntil: "networkidle",
  });
  await page.waitForTimeout(1_500);

  const picker = page.getByRole("radiogroup", { name: /lồng tiếng/i });
  if ((await picker.count()) === 0) {
    problems.push("the player has no voice-over picker for a meeting that has a dub");
  } else {
    // Switching it mutes the recording and starts the dub beside it. Two elements, because a media
    // element has one audio track and a dub has to play *with* the picture.
    await picker.getByRole("radio", { name: /tiếng anh|english/i }).click();
    await page.waitForTimeout(600);

    const state = await page.evaluate(() => {
      const media = [...document.querySelectorAll("audio, video")];
      return media.map((el) => ({ src: el.getAttribute("src") ?? "", muted: el.muted }));
    });
    const dubbed = state.find((el) => el.src.includes("dub-en"));
    if (!dubbed) {
      problems.push(`no element is playing the dub: ${JSON.stringify(state)}`);
    }
    const original = state.find((el) => !el.src.includes("dub-en"));
    if (original && !original.muted) {
      problems.push("the recording is still audible under the dub, so it plays twice");
    }
  }

  await browser.close();
} finally {
  await engine.stop();
}

/** Poll until `check` returns something, or give up with a message a reader can act on. */
async function until(check, ms, complaint) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    const answer = await check();
    if (answer) return answer;
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error(complaint);
}

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const problem of problems) console.error(`  ✗ ${problem}`);
  process.exit(1);
}
console.log("\ndub ok: translated, spoken, written as a lane, and switchable in the player");
