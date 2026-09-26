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

import { daemon as boot } from "./daemon.mjs";
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

const heard = await page.evaluate(() => window.__dub);
await page.screenshot({ path: "/tmp/shots/listen.png" });
await browser.close();

// ---- what happened -------------------------------------------------------

if (heard.length === 0) {
  problems.push("nothing was ever spoken — the dub did not arrive");
} else {
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
const log = engine.log().replace(/\[[0-9;]*m/g, "");
for (const pattern of [/a clause could not be translated/, /a clause could not be spoken/]) {
  const failures = [...log.matchAll(new RegExp(pattern, "g"))].length;
  if (failures > 0) console.log(`  ${failures} clause(s) failed: ${pattern.source}`);
}

await engine.stop();
console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nlisten measured");
process.exit(problems.length ? 1 : 0);
