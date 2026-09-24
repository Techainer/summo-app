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
import { createServer } from "node:net";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { legible } from "./legible.mjs";

const OUT = "/tmp/shots";
mkdirSync(OUT, { recursive: true });

/**
 * A port the operating system says is free, rather than one this file hopes is.
 *
 * It was 4321 with `--strictPort`, and the failure that produced is worth writing down. A vite from
 * an earlier run outlived its suite and kept the port; the new vite therefore died on startup, and
 * `ready()` — which only asks whether *something* answers on 4321 — got its 200 from the dead run's
 * server and carried on. The suite would then photograph a server it did not start, did not
 * configure and cannot reason about, and report "no overflow, contrast AA in both themes" about it.
 *
 * Asking for port 0 and reading back what was bound makes a leaked server impossible to mistake for
 * this one: nothing else is on this port, because it did not exist until a moment ago.
 */
const PORT = await new Promise((resolve, reject) => {
  const probe = createServer();
  probe.on("error", reject);
  probe.listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});

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

/**
 * vite itself, not `npx vite`, and in its own process group.
 *
 * Both of this suite's startup failures were one bug wearing two coats. Spawned through `npx`,
 * vite is a *grandchild*: `SIGTERM` reaches the wrapper, the wrapper exits, and vite keeps running
 * with its stdio pipe still attached to this process — so node's event loop never empties and the
 * suite hangs forever after printing nothing, while leaving a vite behind holding the port. The
 * next run then found that leftover answering on 4321 and photographed it.
 *
 * Running the local binary removes the wrapper, and a process group means the signal reaches
 * everything it started rather than only the thing at the top.
 */
const server = spawn(
  "node_modules/.bin/vite",
  ["--port", String(PORT), "--strictPort", "--host", "127.0.0.1"],
  { stdio: ["ignore", "pipe", "pipe"], detached: true },
);
const log = [];
server.stdout.on("data", (chunk) => log.push(String(chunk)));
server.stderr.on("data", (chunk) => log.push(String(chunk)));

let stopped = false;
const stop = () => {
  if (stopped) return;
  stopped = true;
  // The group, so nothing vite spawned outlives it. `-pid` is the group; `try` because the group
  // is already gone if vite failed to start, and throwing here would mask why.
  try {
    process.kill(-server.pid, "SIGTERM");
  } catch {
    // Already gone.
  }
  // Nothing left to read, and an open pipe on its own is enough to keep node alive.
  server.stdout.destroy();
  server.stderr.destroy();
};
process.on("exit", stop);
// A suite abandoned halfway should not leave a server behind for the next one to mistake for its
// own — which is precisely how this went wrong the first time.
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    stop();
    process.exit(1);
  });
}

/**
 * Wait for vite rather than sleeping: a fixed pause is either too short on CI or wasted locally.
 *
 * Waiting on *our* vite, not on an answer. A server that dies during startup used to leave this
 * loop polling for a minute and then failing with a message about a timeout, which describes the
 * symptom of every possible cause; noticing the process is gone names the actual one and prints
 * what it said on the way out.
 */
async function ready() {
  for (let i = 0; i < 120; i++) {
    if (server.exitCode !== null) {
      console.error(log.join("").slice(-2000));
      throw new Error(`vite exited with ${server.exitCode} before serving anything`);
    }
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

/**
 * Let the page be as tall as its contents, so the picture can contain all of them.
 *
 * `fullPage` grows a shot to the height of the *scrolling document*, and this document does not
 * scroll: `AppShell` scrolls an inner panel, and the body is pinned to the window. Every gallery
 * shot therefore stopped one screenful down with black beneath it — the fields were the last thing
 * in frame, and the chips, checkboxes, segmented controls, status chips, alerts, progress bars,
 * skeletons and card below them were in no picture at all. Shooting the gallery element instead
 * does not help: its ancestors still clip it, so the extra height comes out empty.
 *
 * `legible` never disagreed, because it reads the DOM rather than the pixels — which is exactly why
 * a file called `ui-dark-wide.png`, claiming in its own suite's success line to be every primitive,
 * could show eight of seventeen and nothing fail.
 */
async function unroll(page) {
  await page.evaluate(() => {
    document.documentElement.style.height = "auto";
    document.body.style.height = "auto";
    for (
      let node = document.querySelector('[data-testid="gallery"]');
      node && node !== document.body;
      node = node.parentElement
    ) {
      node.style.height = "auto";
      node.style.maxHeight = "none";
      node.style.overflow = "visible";
    }
  });
  // One frame for the new layout to settle before it is photographed.
  await page.waitForTimeout(200);
}

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

    // Measured first, on the layout the app actually produces, and only then rearranged for the
    // photograph. The other order would check a page this suite had just altered.
    await legible(page, `${scheme}/${shape}`, problems);
    await unroll(page);
    await page.screenshot({ path: `${OUT}/ui-${scheme}-${shape}.png`, fullPage: true });

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
