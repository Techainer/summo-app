/**
 * The draft flow: tinted in the note, confirmed in one gesture.
 *
 * The parts only a browser shows: that unapproved sections are visibly marked and are not also
 * drawn as ordinary content, that selecting a passage offers to refine that passage, and that
 * confirming leaves the text where it was.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "draft" });
const { url: appUrl, port, token, home } = engine;
const meetingId = process.argv[5] ?? "01E2E0";
const notePath =
  process.argv[6] ?? (home ? `${home}/vault/meetings/2026-08-10-hop-dau-tuan.md` : undefined);

// This test confirms the draft, which is what removes the marker — so it seeds its own. Without
// that it passes once and then reports an empty screen forever.
if (notePath) {
  const before = readFileSync(notePath, "utf8");
  if (!before.includes("summo:draft")) {
    writeFileSync(notePath, before.replace("## Tóm tắt", "## Tóm tắt <!-- summo:draft -->"));
    console.log("seeded a draft marker");
  }
}

const browser = await chromium.launch();
// The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
// this the app honours the machine's locale — which is exactly what it should do, and which made
// every assertion here fail the moment translation landed.
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1300, height: 950 },
  colorScheme: "dark",
});
const page = await context.newPage();

const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") problems.push(`console: ${m.text()}`);
});

await page.goto(`${appUrl}?port=${port}&token=${token}#/meetings/${meetingId}`, {
  waitUntil: "networkidle",
});
await page.getByRole("banner", { name: "Thanh trên cùng" }).waitFor({ timeout: 10000 });
await page.locator("h1").waitFor({ timeout: 10000 });
await page.waitForTimeout(900);

const panel = page.getByText("Bản tóm tắt agent viết");
if ((await panel.count()) === 0) problems.push("the draft panel did not render");

// An unapproved section must appear once, in the panel — not also as settled content.
const body = await page.getByRole("main").innerText();
const occurrences = body.split("Chốt ngân sách quý bốn").length - 1;
console.log(`draft text occurrences: ${occurrences}`);
if (occurrences !== 1)
  problems.push(`unapproved text appeared ${occurrences} times, expected once`);

if ((await page.getByRole("button", { name: "Xác nhận" }).count()) === 0)
  problems.push("no confirm button");
if ((await page.getByRole("button", { name: "Bỏ" }).count()) === 0)
  problems.push("no discard button");

// Selecting a passage should offer to rewrite that passage.
await page.evaluate(() => {
  const p = [...document.querySelectorAll("p")].find((el) =>
    el.textContent?.includes("Chốt ngân sách quý bốn"),
  );
  if (!p?.firstChild) return;
  const range = document.createRange();
  range.setStart(p.firstChild, 0);
  // Clamped: the passage only has to be long enough to select part of, and hard-coding 30 threw
  // `IndexSizeError` the moment the seeded summary was shorter than that.
  range.setEnd(p.firstChild, Math.min(30, p.firstChild.textContent?.length ?? 0));
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
  p.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
});
await page.waitForTimeout(400);
const prompt = page.getByRole("textbox", { name: "Muốn sửa thế nào" });
if ((await prompt.count()) === 0) problems.push("selecting a passage did not offer to refine it");
else console.log("refine prompt appeared for the selection");

await page.screenshot({ path: "/tmp/shots/draft.png" });

// Confirming keeps the text and takes the panel away.
await page.getByRole("button", { name: "Xác nhận" }).click();
await page.waitForTimeout(1200);
const after = await page.getByRole("main").innerText();
if (!after.includes("Chốt ngân sách quý bốn")) problems.push("confirming lost the text");
if (after.includes("Bản tóm tắt agent viết")) problems.push("the panel stayed after confirming");
await page.screenshot({ path: "/tmp/shots/draft-confirmed.png" });

await browser.close();
engine.stop();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\ndraft ok");
