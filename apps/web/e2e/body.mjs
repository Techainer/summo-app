/**
 * A page's body, drawn as what it says.
 *
 * Every section of every meeting used to be one paragraph of the file's own bytes. On the screen
 * this app exists to produce, the actions read `- [ ] @Ngọc Chốt spec API`, a link read
 * `[spec](https://…)` and a heading the model wrote read `### Rủi ro`. Nothing caught it: the shape
 * is correct HTML, the contrast passes, there is no overflow and no console error. Only reading it
 * catches it, so this is the suite that reads it.
 *
 * It also drives the two navigations that had nowhere to go — the import panel — and the one choice
 * the window forgot on every launch — the collapsed sidebar.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { chromium } from "playwright";

import { EARLIER, RECENT, boot } from "./daemon.mjs";

const problems = [];
const engine = await boot({ name: "body" });

// A meeting whose summary uses the Markdown a model actually writes.
const RICH = join(engine.home, `vault/meetings/${RECENT}-ban-ke-hoach.md`);
mkdirSync(join(engine.home, "vault/meetings"), { recursive: true });
writeFileSync(
  RICH,
  `---
id: 01BODY
date: ${RECENT}T09:00:00+07:00
duration: 900
participants: ["[[Bạn]]"]
---

# Bản kế hoạch

## Tóm tắt <!-- summo:draft -->
Chốt **ngân sách**, xem [bản spec](https://example.invalid/spec).

- Ngân sách quý bốn đã chốt
- Bản dùng thử gửi thứ Sáu

### Rủi ro
- Thiếu người
- Chậm hàng
  - Nhà cung cấp B

## Việc cần làm
- [ ] @Bạn Gọi nhà cung cấp <!-- id:01BT1 status:todo -->
`,
);

const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 950 },
});
const page = await context.newPage();
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

const open = async (route) => {
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#${route}`, {
    waitUntil: "networkidle",
  });
  await page.waitForTimeout(500);
  for (let i = 0; i < 6; i += 1) {
    const skip = page.getByRole("button", { name: /^Bỏ qua/ });
    if ((await skip.count()) === 0) break;
    await skip
      .first()
      .click({ timeout: 1500 })
      .catch(() => undefined);
    await page.waitForTimeout(150);
  }
};

// ---- the summary is rendered, not printed ---------------------------------
{
  await open("/pages/01BODY");
  const main = page.locator("main");
  await main.getByText("Rủi ro").first().waitFor({ timeout: 10000 });
  const text = await main.innerText();

  // The markers themselves. Any of these on screen means somebody is reading source.
  for (const marker of ["- [ ]", "**ngân sách**", "### Rủi ro", "](https://"]) {
    if (text.includes(marker)) problems.push(`the body shows its own Markdown: ${marker}`);
  }

  if ((await main.locator("li").count()) < 4) {
    problems.push(`a list of risks and actions drew ${await main.locator("li").count()} items`);
  }
  if ((await main.locator("strong", { hasText: "ngân sách" }).count()) === 0) {
    problems.push("bold text was not bold");
  }
  const link = main.locator('a[href="https://example.invalid/spec"]');
  if ((await link.count()) === 0) problems.push("a link in the summary was not a link");
  else if ((await link.getAttribute("rel")) !== "noopener noreferrer") {
    problems.push("a link out of the app carries no rel");
  }
  // The nested bullet keeps its nesting: "Nhà cung cấp B" is under "Chậm hàng", not beside it.
  if ((await main.locator("ul ul li").count()) === 0) problems.push("a nested list was flattened");
}

// ---- the draft is read, not decoded ---------------------------------------
{
  // The draft panel is where a person reads most carefully — it is the text they are being asked
  // to agree to — and it was the one printing `- ` and `**` at them.
  const panel = page.locator("main").locator("section", { hasText: "Tóm tắt" }).first();
  const shown = await panel.innerText();
  for (const marker of ["- Ngân sách", "**ngân sách**", "](https://"]) {
    if (shown.includes(marker)) problems.push(`the draft shows its own Markdown: ${marker}`);
  }
  if ((await panel.locator("li").count()) < 2) problems.push("the draft's list was not a list");

  // And a phrase selected inside the *rendered* text still points at bytes the daemon can find.
  // Without the mapping this comes back as "that passage is no longer in the draft"; with it, the
  // daemon gets past the lookup and fails on the model instead, which is not configured here.
  const selected = await page.evaluate(() => {
    const strong = document.querySelector("main strong");
    const paragraph = strong?.parentElement;
    if (!paragraph) return "";
    const range = document.createRange();
    range.selectNodeContents(paragraph);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    paragraph.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    return selection.toString();
  });
  // The words, spaced as a reader sees them — a bold run must not weld itself to what precedes it.
  if (!selected.includes("Chốt ngân sách")) {
    problems.push(`the rendered text lost its spacing: "${selected}"`);
  }
  await page.waitForTimeout(500);
  const box = page.getByRole("textbox", { name: "Muốn sửa thế nào" });
  if ((await box.count()) === 0) problems.push("selecting a rendered phrase offered no prompt");
  else {
    await box.fill("ngắn hơn");
    await page.getByRole("button", { name: "Sửa", exact: true }).click();
    await page.waitForTimeout(2500);
    const said =
      (await page.locator("main").innerText()) + (await page.locator("body").innerText());
    if (/không còn trong bản nháp|no longer in the draft/i.test(said)) {
      problems.push("the selection did not map back to the file");
    }
    // The refine cannot finish — there is no model here — and the failure must land *on* the page
    // rather than replace it. Every other failure on this screen used to blank it.
    if (!(await page.locator("main").innerText()).includes("Việc cần làm")) {
      problems.push("a failed request took the whole page with it");
    }
    const dismiss = page.getByRole("button", { name: "Đóng" });
    if ((await dismiss.count()) === 0) problems.push("the failure could not be dismissed");
    else await dismiss.first().click();
  }
}

// ---- a checkbox in the body is a checkbox ---------------------------------
{
  const box = page.locator('main input[type="checkbox"]').first();
  await box.waitFor({ timeout: 5000 });
  if (await box.isDisabled()) problems.push("a task with an id could not be ticked");

  await box.click();
  // The daemon rewrites the file and the screen reads it back; until then the tick is optimistic,
  // which is the point — a box that springs back reads as a click that did not land.
  await page.waitForTimeout(300);
  if (!(await box.isChecked())) problems.push("the tick did not show until the disk answered");

  await page.waitForTimeout(2500);
  const file = readFileSync(RICH, "utf8");
  if (!file.includes("- [x] @Bạn Gọi nhà cung cấp")) {
    problems.push("ticking the box did not write the file");
  }
  if (!file.includes("status:done")) problems.push("the task's status was not written");

  // And the board agrees, because both went through the same id.
  const board = await (await fetch(`${engine.url}/tasks?token=${engine.token}`)).json();
  if (!board.done.some((task) => task.id === "01BT1")) {
    problems.push(`the board did not see the tick: ${JSON.stringify(board.done)}`);
  }

  await open("/pages/01BODY");
  const again = page.locator('main input[type="checkbox"]').first();
  await again.waitFor({ timeout: 5000 });
  if (!(await again.isChecked())) problems.push("the tick did not survive a reload");
}

// ---- the import panel is somewhere you can go -----------------------------
{
  await open("/");
  await page.getByRole("button", { name: "Nhập file" }).click();
  await page.waitForTimeout(700);
  if (!page.url().includes("source=upload")) {
    problems.push(`"Nhập file" on the home screen went to ${page.url()}`);
  }
  const shown = await page.locator("main").innerText();
  if (!/mp3/i.test(shown)) problems.push("the import panel did not open");

  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(800);
  if (!/mp3/i.test(await page.locator("main").innerText())) {
    problems.push("a reload threw the import panel away");
  }
}

// ---- a collapsed sidebar stays collapsed ----------------------------------
{
  await open("/");
  await page.getByRole("button", { name: "Ẩn thanh bên" }).click();
  await page.waitForTimeout(400);
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(800);
  if ((await page.getByRole("button", { name: "Hiện thanh bên" }).count()) === 0) {
    problems.push("the sidebar came back after a reload");
  }
}

// ---- a typed note is not a meeting ----------------------------------------
{
  const report = await (
    await fetch(`${engine.url}/report?from=${EARLIER}&to=${RECENT}&token=${engine.token}`)
  ).json();
  const titles = report.meetings.map((m) => m.title);
  if (titles.includes("Ý tưởng giá")) {
    problems.push(`a typed note was counted as a meeting: ${JSON.stringify(titles)}`);
  }
  if (report.without_summary.includes("Ý tưởng giá")) {
    problems.push("the app asked the user to summarise a note they typed");
  }
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("body ok");
