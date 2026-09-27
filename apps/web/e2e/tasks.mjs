/**
 * The board, driven the way a person drives it.
 *
 * The parts that only a browser can check: that dragging a card writes the change back to the
 * Markdown file, that the two boards are genuinely separate, and that the agent's own step list
 * renders rather than collapsing into a spinner.
 */
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "tasks" });
const { url: appUrl, port, token } = engine;

const browser = await chromium.launch();
// The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
// this the app honours the machine's locale — which is exactly what it should do, and which made
// every assertion here fail the moment translation landed.
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1400, height: 900 },
  colorScheme: "dark",
});
const page = await context.newPage();

const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") problems.push(`console: ${m.text()}`);
});

await page.goto(`${appUrl}?port=${port}&token=${token}#/tasks`, { waitUntil: "networkidle" });
await page.getByRole("banner", { name: "Thanh trên cùng" }).waitFor({ timeout: 10000 });
await page.getByRole("heading", { name: "Việc cần làm" }).waitFor({ timeout: 10000 });
await page.waitForTimeout(600);

for (const column of ["Chưa làm", "Đang làm", "Đang chờ", "Xong"]) {
  const count = await page.getByRole("region", { name: column }).count();
  if (count === 0) problems.push(`column missing: ${column}`);
}

const main = page.getByRole("main");
const owners = await main.locator("button[aria-pressed]").allInnerTexts();
console.log(`owner filters: ${owners.join(" | ")}`);
if (!owners.includes("Ngọc")) problems.push(`owner filter missing: ${JSON.stringify(owners)}`);

// Filtering must actually narrow the board.
const before = await page.getByRole("region", { name: "Chưa làm" }).locator("article").count();
await main.getByRole("button", { name: "Ngọc", exact: true }).click();
await page.waitForTimeout(300);
const after = await page.getByRole("region", { name: "Chưa làm" }).locator("article").count();
console.log(`todo before filter: ${before}, after: ${after}`);
if (after >= before) problems.push("filtering by owner did not narrow the column");
await main.getByRole("button", { name: "Tất cả" }).click();
await page.waitForTimeout(300);

await page.screenshot({ path: "/tmp/shots/tasks-people.png" });

// The agent's board is a different shape, with its own plan.
await page.getByRole("radio", { name: "Của agent" }).click();
await page.waitForTimeout(400);
const expand = main.getByRole("button", { name: /Xem \d+ bước|Ẩn các bước/ });
if ((await expand.count()) === 0) problems.push("the agent task showed no step list");
else {
  const label = await expand.first().innerText();
  if (label.startsWith("Xem")) await expand.first().click();
  await page.waitForTimeout(300);
  const steps = await page
    .locator("li")
    .filter({ hasText: /Quét ghi chú|Soạn sự kiện/ })
    .count();
  console.log(`agent steps rendered: ${steps}`);
  if (steps < 2) problems.push(`agent steps did not render: ${steps}`);
}
await page.screenshot({ path: "/tmp/shots/tasks-agent.png" });

// ---- writing one down, changing it, and taking it off ---------------------
//
// None of this existed: every route into the board needed a meeting id, so a task nobody had
// recorded could not be written down at all — "Việc cần làm thì không tạo mới riêng được, cũng
// không action gì được wtf?". A task could be dragged between columns and nothing else.
//
// Driven here rather than against the HTTP routes because the point is the round trip: the browser
// writes, the daemon rewrites a Markdown file in the vault, and the board reads it back.
await page.getByRole("radio", { name: "Bảng" }).click();
await page.waitForTimeout(300);

const NEW = "Gọi ngân hàng về hợp đồng";
await main.getByRole("textbox", { name: "Thêm việc" }).fill(NEW);
await main.getByRole("textbox", { name: "người làm" }).fill("viet");
await main.getByRole("button", { name: "Thêm việc" }).click();
await page.waitForTimeout(800);

const card = main.locator("article").filter({ hasText: NEW });
if ((await card.count()) === 0) {
  problems.push("a task written down with no meeting behind it did not appear on the board");
} else {
  console.log(`created: ${NEW}`);
}

// And it survives a reload, which is the difference between a board and a sketch.
await page.reload({ waitUntil: "networkidle" });
await page.waitForTimeout(800);
if ((await main.locator("article").filter({ hasText: NEW }).count()) === 0) {
  problems.push("the new task did not survive a reload; it never reached the vault");
}

// Rewording. A task read out of a model's summary is sometimes half a sentence, and the only fix
// used to be opening the Markdown file.
const REWORDED = `${NEW} (đã sửa)`;
await main.locator("article").filter({ hasText: NEW }).getByRole("button", { name: "Sửa" }).click();
await main.getByRole("textbox", { name: "Sửa" }).fill(REWORDED);
await main.getByRole("button", { name: "Lưu" }).click();
await page.waitForTimeout(800);
if ((await main.locator("article").filter({ hasText: "đã sửa" }).count()) === 0) {
  problems.push("rewording a task did not take");
} else {
  console.log("reworded");
}

// The list, which is the second way of looking at the same tasks.
await page.getByRole("radio", { name: "Danh sách" }).click();
await page.waitForTimeout(500);
const rows = await main.locator("li").filter({ hasText: REWORDED }).count();
if (rows === 0) problems.push("the list view does not show the task the board does");
else console.log(`list view: the task is there too`);
await page.screenshot({ path: "/tmp/shots/tasks-list.png" });

// Deleting. A line that was never a task could only be dragged to "Xong", which is a lie in the
// one place somebody looks to find out what they finished.
await page.getByRole("radio", { name: "Bảng" }).click();
await page.waitForTimeout(400);
await main
  .locator("article")
  .filter({ hasText: REWORDED })
  .getByRole("button", { name: "Xoá việc" })
  .click();
await main.getByRole("button", { name: "Xoá", exact: true }).click();
await page.waitForTimeout(800);
await page.reload({ waitUntil: "networkidle" });
await page.waitForTimeout(800);
if ((await main.locator("article").filter({ hasText: REWORDED }).count()) > 0) {
  problems.push("the deleted task came back after a reload; it was not removed from the vault");
} else {
  console.log("deleted, and it stayed deleted");
}

await browser.close();
engine.stop();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\ntasks ok");
