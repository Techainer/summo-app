/**
 * Every primitive, in every state, checked the way the screens are.
 *
 * `shots.mjs` photographs screens and asks two questions of each: does it fit, and can it be read.
 * Those are the right questions and the wrong level for a component library — a screen shows each
 * control in whichever state it happens to be in, and nothing in this app is normally rendered
 * busy, or disabled, or holding a sentence long enough to wrap. So those states change without
 * anybody seeing, which is exactly how the eleven hand-copied alert boxes this release removed came
 * to disagree with each other.
 *
 * The page is `/__ui`, added to the router only under `import.meta.env.DEV`. This suite therefore
 * runs against the **development** server rather than the bundled daemon: a release build does not
 * contain the route, and a suite that silently passed because the page 404'd would be worse than no
 * suite at all — so a missing gallery is a failure here, loudly.
 *
 * The checks themselves are `legible.mjs`, the same module `shots.mjs` uses. Two definitions of
 * "AA contrast" would agree until somebody fixed one.
 *
 *     node e2e/ui-shots.mjs
 */
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { legible } from "./legible.mjs";

const OUT = "/tmp/shots";
mkdirSync(OUT, { recursive: true });

const PORT = 4321;

// A daemon, even though the gallery needs nothing from it.
//
// The page is drawn inside the app shell, and the shell's first act is to reach the engine — so
// without one every context filled with `ERR_CONNECTION_REFUSED` and the suite failed on console
// errors that said nothing about any primitive. Booting one is cheaper than teaching this suite to
// ignore a class of error, which is the kind of exception that later hides a real fault.
const engine = await boot(process.argv, { name: "ui-shots", dev: true });

/** Both themes and both shapes, because a primitive can fail in exactly one of the four. */
const VIEWPORTS = [
  ["wide", { width: 1280, height: 900 }],
  ["narrow", { width: 390, height: 844 }],
];
const SCHEMES = ["dark", "light"];

const server = spawn(
  "npx",
  ["vite", "--port", String(PORT), "--strictPort", "--host", "127.0.0.1"],
  { stdio: ["ignore", "pipe", "pipe"] },
);
const log = [];
server.stdout.on("data", (chunk) => log.push(String(chunk)));
server.stderr.on("data", (chunk) => log.push(String(chunk)));

const stop = () => {
  if (!server.killed) server.kill("SIGTERM");
};
process.on("exit", stop);

/** Wait for vite rather than sleeping: a fixed pause is either too short on CI or wasted locally. */
async function ready() {
  for (let i = 0; i < 120; i++) {
    try {
      const response = await fetch(`http://127.0.0.1:${PORT}/`);
      if (response.ok) return;
    } catch {
      // Not listening yet.
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  console.error(log.join("").slice(-2000));
  throw new Error("vite never started");
}

await ready();

const problems = [];
const browser = await chromium.launch();

for (const scheme of SCHEMES) {
  for (const [shape, viewport] of VIEWPORTS) {
    const context = await browser.newContext({ locale: "vi-VN", viewport, colorScheme: scheme });
    await context.addInitScript(() => window.localStorage.setItem("summo.tour", "done"));
    const page = await context.newPage();
    page.on("console", (message) => {
      if (message.type() === "error") problems.push(`${scheme}/${shape}: ${message.text()}`);
    });

    await page.goto(`http://127.0.0.1:${PORT}/?port=${engine.port}&token=${engine.token}#/__ui`, {
      waitUntil: "networkidle",
    });

    // A development build without the route renders the app's not-found screen, and every check
    // below would pass over it happily. The gallery says its own name.
    const there = await page
      .getByTestId("gallery")
      .waitFor({ timeout: 15000 })
      .then(() => true)
      .catch(() => false);
    if (!there) {
      problems.push(
        `${scheme}/${shape}: /__ui did not render — is the dev-only route still added?`,
      );
      await context.close();
      continue;
    }

    // Entrances finish; the picture should be the resting state.
    await page.waitForTimeout(600);
    await page.screenshot({ path: `${OUT}/ui-${scheme}-${shape}.png`, fullPage: true });

    await legible(page, `${scheme}/${shape}`, problems);
    await context.close();
  }
}

await browser.close();
stop();
await engine.stop();

if (problems.length > 0) {
  console.error(`\nPROBLEMS:\n  ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log(`primitives in ${OUT}/ui-*.png, no overflow, contrast AA in both themes`);
