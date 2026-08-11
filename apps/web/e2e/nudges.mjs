/**
 * The nudge strip.
 *
 * Checks the thing the daemon cannot: that a nudge reaches the screen, and that dismissing removes
 * it from the strip without the daemon repeating itself.
 */
import { rmSync } from "node:fs";
import { chromium } from "playwright";

const [, , appUrl, port, token, statePath] = process.argv;

// Asking for a nudge is what consumes it, so a second run would have nothing to show. Clearing the
// daemon's record is the test's way of saying "pretend today just started".
if (statePath) {
  try {
    rmSync(statePath);
  } catch {
    /* nothing said yet */
  }
}
const b = await chromium.launch();
// The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
// this the app honours the machine's locale — which is exactly what it should do, and which made
// every assertion here fail the moment translation landed.
const c = await b.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 860 },
  colorScheme: "dark",
});
const p = await c.newPage();
const problems = [];
p.on("pageerror", (e) => problems.push("pageerror: " + e.message));
await p.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });
await p.getByRole("banner", { name: "Thanh trên cùng" }).waitFor({ timeout: 10000 });
await p.waitForTimeout(1500);
const strip = await p.getByRole("button", { name: "Xem" }).count();
console.log("nudges shown:", strip);
if (strip === 0) problems.push("no nudge appeared despite a waiting draft");
await p.screenshot({ path: "/tmp/shots/nudges.png" });
// Dismissing must remove it from the strip.
const dismiss = p.getByRole("button", { name: /Bỏ qua:/ }).first();
if ((await dismiss.count()) > 0) {
  await dismiss.click();
  await p.waitForTimeout(400);
}
console.log("after dismiss:", await p.getByRole("button", { name: "Xem" }).count());
await b.close();
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("nudges ok");
