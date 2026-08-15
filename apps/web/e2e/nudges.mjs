/**
 * The nudge strip.
 *
 * Checks the thing the daemon cannot: that a nudge reaches the screen, and that dismissing removes
 * it from the strip without the daemon repeating itself.
 */
import { rmSync } from "node:fs";
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "nudges" });
const { url: appUrl, port, token, home } = engine;
const statePath = process.argv[5] ?? (home ? `${home}/nudges.json` : undefined);

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
const strip = await p.getByRole("button", { name: /^Bỏ qua:/ }).count();
console.log("nudges shown:", strip);
if (strip === 0) problems.push("no nudge appeared despite a waiting draft");
// One bar, whatever is waiting behind it.
//
// Each nudge used to draw its own full-width strip and nothing bounded how many: a Monday with a
// draft, an overdue task and a weekly summary opened the app with three of them stacked above the
// content. This is the assertion that keeps the notice from growing back into a header.
if (strip > 1) problems.push(`${strip} nudges are stacked above the app; only one should be`);
await p.screenshot({ path: "/tmp/shots/nudges.png" });

// The rest are counted, not dropped — and the count opens them.
const more = p.getByRole("button", { name: /còn \d+ nữa/ });
if ((await more.count()) > 0) {
  const label = await more.innerText();
  await more.click();
  await p.waitForTimeout(400);
  const opened = await p.getByRole("button", { name: /^Bỏ qua:/ }).count();
  console.log(`"${label}" opened ${opened}`);
  if (opened < 2) problems.push(`"${label}" was offered and opened ${opened} nudges`);
  await p.screenshot({ path: "/tmp/shots/nudges-all.png" });
} else {
  console.log("only one nudge was due, so there was nothing to count");
}

// Dismissing must remove it from the strip.
const dismiss = p.getByRole("button", { name: /^Bỏ qua:/ }).first();
// By its label rather than its text: the button reads "✕", and what identifies which nudge it
// belongs to is the accessible name — "Bỏ qua: <title>" — which is also the only thing a screen
// reader has to tell three of these apart.
const before = await dismiss.getAttribute("aria-label").catch(() => null);
if ((await dismiss.count()) > 0) {
  await dismiss.click();
  await p.waitForTimeout(400);
}
console.log("after dismiss:", await p.getByRole("button", { name: /^Bỏ qua:/ }).count());
if (before && (await p.getByRole("button", { name: before, exact: true }).count()) > 0) {
  problems.push(`"${before}" was dismissed and is still on the strip`);
}
await b.close();
engine.stop();
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("nudges ok");
