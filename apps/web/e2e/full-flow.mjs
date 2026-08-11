/**
 * The whole product, driven the way a person drives it.
 *
 * Chromium is given a WAV file as its microphone, so this exercises the real path end to end:
 * getUserMedia → the capture worklet's resampling → the WebSocket → the daemon → voice detection →
 * decoding → events → React → the file on disk. Every piece is unit-tested; only this catches them
 * being wired together wrongly.
 */
import { chromium } from "playwright";
import fs from "node:fs";

const [, , appUrl, port, token, wav] = process.argv;

const browser = await chromium.launch({
  args: [
    "--use-fake-ui-for-media-stream",
    "--use-fake-device-for-media-stream",
    `--use-file-for-fake-audio-capture=${wav}`,
    "--autoplay-policy=no-user-gesture-required",
  ],
});
// The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
// this the app honours the machine's locale — which is exactly what it should do, and which made
// every assertion here fail the moment translation landed.
const context = await browser.newContext({
  locale: "vi-VN",
  permissions: ["microphone"],
  viewport: { width: 1180, height: 760 },
  colorScheme: "dark",
});
const page = await context.newPage();

const problems = [];
page.on("console", (m) => {
  if (m.type() === "error") problems.push(`console: ${m.text()}`);
});
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });

console.log("clicking record…");
await page.getByRole("button", { name: /Bắt đầu ghi/ }).click();

// Wait for the first committed line rather than a fixed sleep: the assertion is that text arrives,
// and a timeout here is the failure worth reporting.
await page.locator(".line").first().waitFor({ timeout: 60000 });
await page.waitForTimeout(12000);

const lines = await page.locator(".line-text").allInnerTexts();
await page.screenshot({ path: "/tmp/shots/recording.png" });

// The compact window is what sits on top of a call, so check it renders while recording.
await page.getByRole("button", { name: /Thu gọn/ }).click();
await page.waitForTimeout(300);
await page.screenshot({ path: "/tmp/shots/compact.png" });
await page.getByRole("button", { name: /Mở rộng/ }).click();

console.log("clicking stop…");
await page.getByRole("button", { name: /Dừng ghi/ }).click();
await page.waitForTimeout(3000);
const notice = await page
  .locator(".notice")
  .innerText()
  .catch(() => "");
await page.screenshot({ path: "/tmp/shots/stopped.png" });

await browser.close();

console.log(`\ntranscript lines on screen: ${lines.length}`);
for (const line of lines.slice(0, 8)) console.log(`  ${line}`);
console.log(`\nstatus bar after stop: ${notice}`);
console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nno console errors");

if (lines.length === 0) {
  console.log("\nFAIL: no transcript reached the screen");
  process.exit(1);
}
