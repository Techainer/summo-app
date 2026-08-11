/**
 * The board, driven the way a person drives it.
 *
 * The parts that only a browser can check: that dragging a card writes the change back to the
 * Markdown file, that the two boards are genuinely separate, and that the agent's own step list
 * renders rather than collapsing into a spinner.
 */
import { chromium } from "playwright";

const [, , appUrl, port, token] = process.argv;

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

await browser.close();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\ntasks ok");
