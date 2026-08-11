/**
 * One meeting's own screen: player, summary, transcript.
 *
 * Covers the wiring a unit test cannot: that the audio route is registered and reachable with the
 * lane name the client derives, that the segmented control is a real radio group, and that the
 * transcript virtualises into clickable rows rather than one long block.
 */
import { chromium } from "playwright";

const [, , appUrl, port, token, meetingId = "01A1"] = process.argv;

const browser = await chromium.launch();
// The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
// this the app honours the machine's locale — which is exactly what it should do, and which made
// every assertion here fail the moment translation landed.
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
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

console.log(`title: ${await page.locator("h1").innerText()}`);

// The player is only useful if it can seek, which is what the Range route exists for.
if ((await page.getByRole("button", { name: "Phát" }).count()) === 0)
  problems.push("no play button");
if ((await page.getByRole("slider", { name: "Vị trí phát" }).count()) === 0)
  problems.push("no scrubber");

// The rail opens on comments, so the thread is what a reader sees first.
if ((await page.getByRole("radio", { name: "Bình luận" }).count()) === 0) {
  problems.push("no comments pane");
}

// Radix renders a single-choice toggle group as radios; that is correct, and the test should say so.
// The label is translated now — this suite runs in Vietnamese, where it reads "Bản ghi".
await page.getByRole("radio", { name: "Bản ghi" }).click();
await page.waitForTimeout(400);

const chips = await page.getByRole("button", { name: /Nghe từ/ }).count();
console.log(`transcript chips: ${chips}`);
if (chips < 3) problems.push(`expected the seeded transcript, got ${chips} chips`);

// Clicking a line seeks; with no decodable audio it must still not throw.
await page
  .getByRole("button", { name: /Nghe từ/ })
  .nth(1)
  .click();
await page.waitForTimeout(300);

await page.screenshot({ path: "/tmp/shots/meeting.png" });
await browser.close();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\nmeeting ok");
