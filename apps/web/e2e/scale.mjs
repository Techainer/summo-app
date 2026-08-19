/**
 * The library on a vault that has been used for a year.
 *
 * Every other suite here runs against three documents, which is the vault of somebody who installed
 * the app this morning. Seeded with a thousand instead, two things went wrong that nothing in the
 * repository could have caught:
 *
 * - **The list was invisible.** Rows start at `opacity: 0` and are staggered in. The step was a
 *   constant, so the delay grew with the list: the eight hundredth row was still transparent ten
 *   seconds after the library opened, and the vault took half a minute to finish appearing.
 * - **Everything was drawn.** 5,000 meetings put 125,000 nodes in the document and took 5.6 seconds
 *   from the click to the first row, on an idle machine.
 *
 * So this suite asserts the two properties that keep both fixed: what is drawn is bounded, and what
 * is drawn is *visible* — quickly, and without anybody scrolling to make it so.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

/**
 * How many meetings to seed.
 *
 * A year of daily meetings, and enough to be several times any sane window. Writing them takes
 * about a second; the point is the vault, not the volume.
 */
const MEETINGS = 600;

/** What the component draws before asking to be scrolled — `PAGE` in `Library.tsx`. */
const PAGE = 120;

const engine = await daemon(process.argv, { name: "scale" });
const { url: appUrl, port, token, home } = engine;

if (home) {
  const meetings = join(home, "vault/meetings");
  for (let i = 0; i < MEETINGS; i++) {
    const folder = join(meetings, `phong-${i % 8}`);
    mkdirSync(folder, { recursive: true });
    const day = String(1 + (i % 28)).padStart(2, "0");
    writeFileSync(
      join(folder, `2026-07-${day}-hop-${i}.md`),
      `---\nid: 01S${String(i).padStart(5, "0")}\ndate: 2026-07-${day}T09:00:00+07:00\nduration: ${600 + i}\nparticipants: ["[[Bạn]]", "[[Ngọc]]"]\ntags: [weekly]\ncolor: teal\n---\n\n# Họp số ${i}\n\n## Tóm tắt\nChốt việc ${i}.\n\n## Transcript\n**[00:01:00] Bạn** — Dòng ${i} về ngân sách <!-- seq:0 end:60.0 -->\n`,
    );
  }
}

const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
  colorScheme: "dark",
});
const page = await context.newPage();
const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
page.on("console", (m) => m.type() === "error" && problems.push(`console: ${m.text()}`));

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });

const started = Date.now();
await page.getByRole("button", { name: "Đã lưu" }).click();
await page.locator('[data-testid="meeting-row"]').first().waitFor({ timeout: 60_000 });
await page.waitForLoadState("networkidle");
const opened = Date.now() - started;

const rows = () => page.locator('[data-testid="meeting-row"]').count();
const drawn = await rows();
const nodes = await page.evaluate(() => document.querySelectorAll("*").length);
console.log(`${MEETINGS} meetings: opened in ${opened}ms · ${drawn} rows drawn · ${nodes} nodes`);

// A window, not the vault. The bound is generous — what it rules out is the version that drew
// every row, which on this vault is five times this and on a real one is unbounded.
if (drawn > PAGE + 8) {
  problems.push(`${drawn} of ${MEETINGS} rows were drawn at once; the window is ${PAGE}`);
}

// And drawn *visibly*.
//
// Measured with the list grouped by week, because the stagger runs per group: seeded across a
// month, the default day grouping puts twenty rows in each section and no delay is long enough to
// see. A week — or a folder, or a day somebody had eight meetings — is one section holding the
// whole window, which is where the constant step showed itself. A second is longer than the entire
// assembly is allowed to take, so a row still transparent here is one waiting in a queue.
await page.getByRole("button", { name: "Tuần", exact: true }).click();
await page.locator('[data-testid="meeting-row"]').first().waitFor({ timeout: 30_000 });
await page.waitForTimeout(1000);
const transparent = await page.evaluate(
  () =>
    [...document.querySelectorAll('[data-testid="meeting-row"]')].filter(
      (row) => Number(getComputedStyle(row).opacity) < 0.9,
    ).length,
);
console.log(`still transparent a second after opening: ${transparent}`);
if (transparent > 0) {
  problems.push(`${transparent} rows were drawn but invisible a second after the library opened`);
}
await page.screenshot({ path: "/tmp/shots/scale-library.png" });

// The rest are reachable, and the button says how many are left rather than hiding the fact.
const more = page.getByTestId("library-more");
if ((await more.count()) === 0) {
  problems.push("the list was cut short and nothing on screen offered the rest");
} else {
  const label = await more.innerText();
  console.log(`the list offers: "${label}"`);
  const left = Number(label.replace(/\D+/g, ""));
  // Against the daemon's own count rather than against `MEETINGS`: the harness seeds documents of
  // its own, and a suite that hardcodes the arithmetic goes red when it seeds one more.
  const view = await fetch(`${appUrl}/library`, {
    headers: { authorization: `Bearer ${token}` },
  }).then((r) => r.json());
  if (left !== view.total - drawn) {
    problems.push(`the button offers ${left} more, but ${view.total - drawn} are undrawn`);
  }
  await more.click();
  await page.waitForTimeout(500);
  const after = await rows();
  console.log(`after asking for more: ${after} rows`);
  if (after <= drawn) problems.push(`asking for more drew ${after} rows, up from ${drawn}`);
}

// Scrolling the list is the other way to ask, and the one a person actually uses.
const before = await rows();
await page.locator('[data-testid="meeting-list"]').first().hover();
await page.mouse.wheel(0, 30_000);
await page.waitForTimeout(800);
const scrolled = await rows();
console.log(`after scrolling to the bottom: ${scrolled} rows`);
if (scrolled <= before) {
  problems.push(
    `scrolling to the bottom of ${before} rows drew ${scrolled}; the list did not grow`,
  );
}

await browser.close();
engine.stop();
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("scale ok");
