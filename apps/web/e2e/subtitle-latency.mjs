/**
 * How long a subtitle takes, under the condition that makes batching visible.
 *
 * `in-meeting.mjs` measures one subtitle, on `vi-paced.wav` — three-second gaps, so a translation
 * batch never fills and every line takes the send-immediately path. That is the right fixture for
 * what that suite checks and the wrong one for this question: the argument about grouping lines
 * into one request only bites when lines arrive faster than the model answers, and a paced clip
 * cannot produce that.
 *
 * So: continuous speech, two subtitle languages, and every line timed rather than one. The output
 * is numbers, not a pass mark — the only thing that fails here is a subtitle that never arrives,
 * because a latency suite that fails on a threshold becomes a suite people re-run until it passes.
 *
 * It also reports the batch sizes the daemon actually used. The whole argument turns on that number
 * and nothing printed it; reasoning about it from the constants is guessing, because it depends on
 * whether the model is losing to the speaker on this machine.
 *
 * ```bash
 * node e2e/subtitle-latency.mjs
 * ```
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

/**
 * Eight FLEURS clips, 0.45 s apart — long enough for the detector to close each utterance, short
 * enough that the next one is already being spoken while the last is still being translated.
 */
const wav = join(HERE, "fixtures/vi-continuous.wav");

const problems = [];
const MODELS = ["gipformer-65m", "silero-vad-v5", "small100"];

const local = await mirror(MODELS, { name: "subtitle-latency" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this measures translation; it means nothing without a translator");
  process.exit(1);
}

const engine = await boot(process.argv, {
  name: "subtitle-latency",
  registry: local.registry,
  // `summo_engine::live` logs one line per run with the batch size in it.
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
    // `model`, not `id` — a job carries its own id and the model it is fetching, and matching on
    // the wrong one finds nothing forever rather than failing.
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

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });

// Both targets before recording starts, so every line of the meeting is a live line. Turning
// translation on mid-meeting fills the backlog in, and the backlog is deliberately slower than the
// live path — timing against it would measure the wrong thing.
await page.evaluate(() => {
  const stored = JSON.parse(localStorage.getItem("summo.capture") ?? "{}");
  localStorage.setItem("summo.capture", JSON.stringify({ ...stored, translateInto: ["en", "ja"] }));
});
await page.reload({ waitUntil: "networkidle" });

await page
  .getByTestId("home")
  .getByRole("button", { name: /Bắt đầu ghi/ })
  .click();

/**
 * When each line settled and when each of its subtitles first appeared, read off the DOM.
 *
 * Polled rather than observed through an event, because what is being timed is when a person could
 * *read* it — a translation delivered to a store and not yet painted is not a subtitle.
 *
 * Timed from the line going **final**, not from it first appearing. Only a final segment is
 * queued for translation: a partial is about to change, and paying to translate it would put a
 * subtitle under words the speaker has not finished saying. Timing from the partial would fold the
 * rest of the sentence into the number and call it latency.
 *
 * Keyed by position. The lines only ever append — a partial is replaced in place by its final — so
 * the nth row stays the nth row, and there is no id in the DOM to key on instead.
 */
const seen = new Map();
const started = Date.now();

const poll = async () => {
  const rows = await page.$$eval('[data-testid="transcript-line"]', (nodes) =>
    nodes.map((node) => ({
      final: node.getAttribute("data-source") !== "partial",
      subs: [
        ...(node.parentElement?.querySelectorAll('[data-testid="transcript-translation"]') ?? []),
      ].map((n) => n.getAttribute("lang") ?? "?"),
    })),
  );
  const now = Date.now();
  rows.forEach(({ final, subs }, index) => {
    const entry = seen.get(index) ?? { finalAt: null, subs: new Map() };
    if (final && entry.finalAt === null) entry.finalAt = now;
    for (const lang of subs) if (!entry.subs.has(lang)) entry.subs.set(lang, now);
    seen.set(index, entry);
  });
};

// The clip is 58 seconds; a little past it, so the last utterance's subtitle has somewhere to land.
while (Date.now() - started < 75_000) {
  await poll();
  await page.waitForTimeout(100);
}

await page.screenshot({ path: "/tmp/shots/subtitle-latency.png" });
await browser.close();

// ---- what happened -------------------------------------------------------

// A subtitle that was already painted in the same poll as the line going final reads as 0 ms and
// is a real answer, not a missing one: the poll is 100 ms wide. A subtitle with no final line
// behind it is dropped — it cannot be timed against anything.
const lags = [];
for (const [, entry] of seen) {
  if (entry.finalAt === null) continue;
  for (const [lang, at] of entry.subs) lags.push({ lang, ms: Math.max(0, at - entry.finalAt) });
}

if (lags.length === 0) {
  problems.push("no subtitle ever appeared — nothing was measured");
} else {
  const sorted = [...lags].map((l) => l.ms).sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  console.log(
    `\nsubtitle lag over ${lags.length} subtitles on ${seen.size} lines, two targets:\n` +
      `  median ${(median / 1000).toFixed(2)}s   worst ${(sorted.at(-1) / 1000).toFixed(2)}s   ` +
      `best ${(sorted[0] / 1000).toFixed(2)}s`,
  );

  // Line by line, because an average hides which one was slow and that is the whole question. A
  // first line far above the rest is a model being loaded, not a queue; a slow one in the middle is
  // a queue. The two have different fixes and a summary statistic cannot tell them apart.
  const perLine = [...seen.entries()]
    .filter(([, entry]) => entry.finalAt !== null && entry.subs.size > 0)
    .map(([index, entry]) => {
      const worst = Math.max(...[...entry.subs.values()].map((at) => at - entry.finalAt));
      return `${index + 1}:${(Math.max(0, worst) / 1000).toFixed(1)}s`;
    });
  console.log(`  per line — ${perLine.join("  ")}`);
}

// The batch sizes the daemon really used. Colour is stripped first: `tracing` writes the field
// name, an escape, then `=`, so a plain search for `lines=` finds nothing while the lines are
// sitting in the buffer. That mistake cost a release's worth of "the daemon logged no decode".
const log = engine.log().replace(/\[[0-9;]*m/g, "");
const runs = [...log.matchAll(/translating a run lines=(\d+) targets=(\d+) grouped=(\w+)/g)].map(
  (m) => ({ lines: Number(m[1]), targets: Number(m[2]), grouped: m[3] === "true" }),
);

if (runs.length === 0) {
  console.log("\nbatch sizes: not measured — the daemon logged no run");
} else {
  const sizes = runs.map((r) => r.lines);
  const biggest = Math.max(...sizes);
  console.log(
    `\n${runs.length} translation runs, ${sizes.reduce((a, b) => a + b, 0)} lines:\n` +
      `  largest run ${biggest} line${biggest === 1 ? "" : "s"}, ` +
      `grouped=${runs[0].grouped}`,
  );
  // The claim this file exists to check, stated so a future reader sees which way it fell.
  console.log(
    biggest > 1
      ? "  → lines do queue here, so answering per line is what keeps the first one early"
      : "  → the model keeps up with the speaker, so every run is a single line either way",
  );
}

await engine.stop();
console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nlatency measured");
process.exit(problems.length ? 1 : 0);
