/**
 * The daemon going away under an open app.
 *
 * It can: it crashes, it is quit from the tray, the machine sleeps and comes back with the port
 * gone. Driven for the first time, the app did not notice. The layout stayed, the meetings already
 * fetched stayed on screen, and a note could be typed into for four seconds — every request failing
 * with `ERR_CONNECTION_REFUSED` in a console no user reads — with nothing said and nothing saved.
 *
 * So: kill it, and require the app to say so.
 */
import { execSync } from "node:child_process";

import { chromium } from "playwright";

import { boot } from "./daemon.mjs";

/** Two misses at a second and a half, plus the poll that was already in flight. */
const NOTICE_MS = 12_000;

const engine = await boot({ name: "offline" });
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
});
const page = await context.newPage();
const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
  waitUntil: "networkidle",
});

// Nothing to report while it is up. A bar that is always there is a bar nobody reads.
await page.waitForTimeout(2000);
if ((await page.getByTestId("unreachable").count()) > 0) {
  problems.push("the app said it could not reach a daemon that was answering");
}

execSync(`fuser -k -KILL ${engine.port}/tcp 2>/dev/null || true`);
console.log(`killed the daemon on port ${engine.port}`);

await page
  .getByTestId("unreachable")
  .waitFor({ timeout: NOTICE_MS })
  .catch(() =>
    problems.push(`the daemon was gone for ${NOTICE_MS / 1000}s and the app never said so`),
  );

if ((await page.getByTestId("unreachable").count()) > 0) {
  const said = (await page.getByTestId("unreachable").innerText()).replace(/\s+/g, " ");
  console.log(`the app says: ${said}`);
  // The sentence has to be about the user's work, not about a socket. "Disconnected" tells somebody
  // nothing about whether the paragraph they just typed still exists.
  if (!/chưa được lưu/.test(said)) {
    problems.push(`the notice does not say what it means for what was typed: "${said}"`);
  }
  if ((await page.getByRole("button", { name: "Thử lại" }).count()) === 0) {
    problems.push("the notice offers no way to try again");
  }
}

// The app is still readable: what is on screen is the only copy of anything unsaved, so the notice
// must not replace it.
const body = await page.locator("body").innerText();
if (!/Kho/.test(body)) problems.push("the app replaced itself with the notice");
await page.screenshot({ path: "/tmp/shots/offline.png" });

await browser.close();
engine.stop();
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("offline ok");
