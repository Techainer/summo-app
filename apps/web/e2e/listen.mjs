/**
 * Hearing the meeting in another language, measured end to end.
 *
 * The one number that matters for a live dub is how long after somebody speaks you hear it, and no
 * amount of unit testing produces it: the path runs through a recogniser, a clause committer, a
 * translation model, a synthesiser and a socket, and every one of them is a real thing on a real
 * machine.
 *
 * So this records Vietnamese, asks to hear it in English, and times the gap from a line appearing
 * on screen to the first dubbed sample for that line arriving. The audio is intercepted on the
 * socket rather than played — a headless browser has no speakers, and what is being measured is
 * when the sound was *available*, which is the part the daemon controls.
 *
 * Prints numbers rather than asserting them, for the reason `subtitle-latency.mjs` gives: a latency
 * threshold on a shared runner becomes a test people re-run until it goes green. The one thing that
 * fails here is a dub that never arrives at all.
 *
 * ```bash
 * node e2e/listen.mjs
 * ```
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { daemon as boot, plain } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-continuous.wav");

const problems = [];
const MODELS = ["gipformer-65m", "silero-vad-v5", "small100", "vits-en-ljspeech"];

const local = await mirror(MODELS, { name: "listen" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this measures a spoken translation; it means nothing without a voice");
  process.exit(1);
}

const engine = await boot(process.argv, {
  name: "listen",
  registry: local.registry,
  log: "summo_engine=debug,info",
});
const { url: appUrl, port, token } = engine;
const at = (path) => `${appUrl}${path}${path.includes("?") ? "&" : "?"}token=${token}`;

async function install(id) {
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 900; i++) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((candidate) => candidate.model === id);
    if (job?.state === "done") return;
    if (job?.state === "failed") throw new Error(`${id}: ${job.error ?? "install failed"}`);
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`${id}: still installing after 450s`);
}

for (const id of MODELS) {
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
const context = await browser.newContext({
  // Animations off, and not only to stop this racing them.
  //
  // Playwright waits for an element to be "stable" before clicking it, and a spring that is still
  // settling never is. On a loaded runner the PiP panel's entrance animation outlived the wait and
  // the suite failed on a button that was on screen the whole time — a flake that only appears
  // under load, which is the worst kind to chase.
  //
  // The app gates every animation on `motion-safe`, so this exercises the reduced-motion path at
  // the same time. Nothing covered it before.
  reducedMotion: "reduce",
  locale: "vi-VN",
  permissions: ["microphone"],
  viewport: { width: 1180, height: 900 },
  colorScheme: "dark",
});
const page = await context.newPage();
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

/**
 * Watch the socket rather than the speakers.
 *
 * A headless browser plays nothing, and a test that asserted on `AudioContext` would be measuring
 * the browser's scheduler. The frame arriving is the moment the dub became available, which is the
 * part of the path this code owns.
 */
await page.addInitScript(() => {
  const heard = [];
  Object.defineProperty(window, "__dub", { get: () => heard });
  const Native = window.WebSocket;
  window.WebSocket = class extends Native {
    constructor(...args) {
      super(...args);
      this.addEventListener("message", (event) => {
        if (!(event.data instanceof ArrayBuffer) || event.data.byteLength < 14) return;
        const view = new DataView(event.data);
        if (view.getUint8(0) !== 0x01) return;
        const head = 2 + view.getUint8(1);
        heard.push({
          at: Date.now(),
          seq: Number(view.getBigUint64(head + 4, true)),
          // Four bytes a sample, at the rate in the frame.
          seconds: (event.data.byteLength - head - 12) / 4 / view.getUint32(head, true),
        });
      });
    }
  };
});

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });

// English subtitles, and hear them. Both before recording starts, so every line is a live line.
await page.evaluate(() => {
  const stored = JSON.parse(localStorage.getItem("summo.capture") ?? "{}");
  localStorage.setItem(
    "summo.capture",
    JSON.stringify({ ...stored, translateInto: ["en"], listenIn: "en", listenVolume: 1 }),
  );
});
await page.reload({ waitUntil: "networkidle" });

await page
  .getByTestId("home")
  .getByRole("button", { name: /Bắt đầu ghi/ })
  .click();

// When each line first appeared on screen, and when it settled. The dub commits clauses from
// partials, so the honest thing to time against is the line *appearing* — that is when the words
// were being said.
const lines = new Map();
const started = Date.now();
while (Date.now() - started < 80_000) {
  const rows = await page.$$eval('[data-testid="transcript-line"]', (nodes) =>
    nodes.map((node) => node.getAttribute("data-source") !== "partial"),
  );
  const now = Date.now();
  rows.forEach((final, index) => {
    const entry = lines.get(index) ?? { seen: now, finalAt: null };
    if (final && entry.finalAt === null) entry.finalAt = now;
    lines.set(index, entry);
  });
  await page.waitForTimeout(100);
}

// Whether the app is showing its own failure screen rather than the meeting.
const crashed = await page
  .locator("text=/Something went wrong|Đã xảy ra lỗi/i")
  .first()
  .textContent({ timeout: 500 })
  .catch(() => null);

const heard = await page.evaluate(() => window.__dub);
await page.screenshot({ path: "/tmp/shots/listen.png" });

// And the panel a person opens mid-meeting to change any of this, which is where the control for
// something audible has to be.
// The meeting's bar opens its panel by default — `LiveBar` passes `expanded`, so the controls are
// already on screen and the button beside them says "done", not "change". A first version of this
// clicked that button, closed the panel it meant to photograph, and reported the controls missing.
const panel = page.getByTestId("live-bar");
if ((await panel.getByLabel(/đọc thành tiếng|read aloud/i).count()) === 0) {
  problems.push("the meeting bar has no way to stop the voice that is playing");
}
if ((await panel.getByTestId("listen-volume").count()) === 0) {
  problems.push("the voice is playing with no volume control beside it");
}

await page.screenshot({ path: "/tmp/shots/listen-panel.png" });
for (const width of [390, 768]) {
  await page.setViewportSize({ width, height: 900 });
  await page.waitForTimeout(400);
  await page.screenshot({ path: `/tmp/shots/listen-panel-${width}.png` });

  // The bar exists to answer "is it recording, and can it hear me". At 390 px the row used to
  // squeeze until the label, the clock and the buttons drew over each other — nothing overflowed
  // the viewport, so the overflow guard saw nothing and only a screenshot did. These are the two
  // things that must survive the squeeze.
  const box = await panel
    .getByText(/^Đang ghi$/)
    .first()
    .boundingBox()
    .catch(() => null);
  if (!box || box.width < 40 || box.height < 8) {
    problems.push(
      `at ${width}px the recording label is ${box ? `${Math.round(box.width)}px` : "gone"}`,
    );
  }
  const clock = await panel
    .getByLabel(/Thời gian|Elapsed/i)
    .first()
    .boundingBox()
    .catch(() => null);
  if (box && clock && box.y === clock.y && box.x + box.width > clock.x) {
    problems.push(`at ${width}px the label and the clock overlap`);
  }
}
await page.setViewportSize({ width: 1180, height: 900 });
await browser.close();

// The daemon writes its closing lines when it notices the socket has gone, which is after this
// process has already closed it. Reading the log immediately is a race, and losing it looks
// exactly like a daemon that reported nothing.
await new Promise((resolve) => setTimeout(resolve, 2000));

// ---- what happened -------------------------------------------------------

// Which layer failed, said before anything else. "No dub" is the symptom of a silent
// recogniser as often as of a broken dub, and a suite that only reports the last stage sends
// somebody to read the wrong module — it did exactly that once.
console.log(`\ntranscript: ${lines.size} line(s)`);

// An app that has crashed is not an app that transcribed nothing, and it reads identically from
// out here. React's error boundary catches the throw, so `pageerror` never fires and the only sign
// is a screen saying so — which this suite happily reported as a silent recogniser, sending me to
// read the wrong module for the third time in one night.
if (crashed) {
  problems.push(`the interface crashed: ${crashed}`);
} else if (lines.size === 0) {
  problems.push("nothing was transcribed — the failure is upstream of the dub");
} else if (heard.length === 0) {
  problems.push("nothing was ever spoken — the dub did not arrive");
}

// The daemon's own account, whichever layer failed. Silence is the symptom of a dozen different
// faults and a suite reporting only the symptom is what makes each of them cost an hour.
if (problems.length > 0) {
  console.log("--- daemon log, last 4000 characters ---");
  console.log(plain(engine).slice(-4000));
}

if (heard.length > 0) {
  // One entry per utterance: the first chunk for a sequence number is when the listener started
  // hearing that line. Later chunks for it are the rest of the sentence.
  const first = new Map();
  for (const chunk of heard) if (!first.has(chunk.seq)) first.set(chunk.seq, chunk.at);

  const ordered = [...lines.entries()].sort(([a], [b]) => a - b);
  const lags = [];
  [...first.entries()]
    .sort(([a], [b]) => a - b)
    .forEach(([, at], index) => {
      const line = ordered[index]?.[1];
      if (!line) return;
      lags.push({ fromSeen: at - line.seen, fromFinal: line.finalAt ? at - line.finalAt : null });
    });

  const spoken = heard.reduce((total, chunk) => total + chunk.seconds, 0);
  console.log(
    `\n${heard.length} chunks over ${first.size} utterances, ${spoken.toFixed(1)}s of speech`,
  );

  if (lags.length > 0) {
    const show = (key) => {
      const values = lags.map((l) => l[key]).filter((v) => v !== null);
      if (values.length === 0) return "not measured";
      const sorted = [...values].sort((a, b) => a - b);
      return (
        `median ${(sorted[Math.floor(sorted.length / 2)] / 1000).toFixed(2)}s  ` +
        `worst ${(sorted.at(-1) / 1000).toFixed(2)}s`
      );
    };
    // Both, because they answer different questions. From the line settling is the comparison
    // against the subtitle; from the line first appearing is what a listener experiences, and it is
    // the one the clause path exists to shrink.
    console.log(`  after the line settled  : ${show("fromFinal")}`);
    console.log(`  after the words appeared: ${show("fromSeen")}`);
  }
}

// What the daemon thought it was doing, in its own words.
const log = plain(engine);
// The whole thing on disk, because a grep for the one line you expected is how you miss the line
// that explains why it is not there.
(await import("node:fs")).writeFileSync("/tmp/listen-daemon.log", log);

// The decisions the dub made, split by cause. Without this the only number is "how many chunks
// arrived", which cannot tell a voice that is busy from a listener who is too far behind — two
// faults with opposite fixes.
const tally = log.match(
  /live dub finished committed=(\d+) spoken=(\d+) dropped_busy=(\d+) dropped_behind=(\d+) revisions=(\d+)/,
);
if (tally) {
  const [, committed, spoken, busy, behind, revisions] = tally.map(Number);
  console.log(
    `\n${committed} pieces settled: ${spoken} spoken, ${busy} dropped (voice busy), ` +
      `${behind} dropped (too far behind)`,
  );
  console.log(`  ${revisions} time(s) the final contradicted something already said`);
} else {
  console.log("\npieces: not measured — the daemon logged no tally");
}
for (const pattern of [/a clause could not be translated/, /a clause could not be spoken/]) {
  const failures = [...log.matchAll(new RegExp(pattern, "g"))].length;
  if (failures > 0) console.log(`  ${failures} clause(s) failed: ${pattern.source}`);
}

await engine.stop();
console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nlisten measured");
process.exit(problems.length ? 1 : 0);
