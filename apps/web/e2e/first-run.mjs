/**
 * A clean install, driven the way a new user drives it.
 *
 * Every other suite runs against a seeded vault and a daemon somebody already started. This one
 * covers the thing they all assume: that a person who has just downloaded one binary can get from
 * nothing to a working app. It is the story the product is sold on, and until this existed it was
 * the only story with no test.
 *
 * It runs against the *bundled* binary — interface compiled in, served by the daemon itself — which
 * is a different code path from the dev server the other suites use. Three things only break here:
 * the asset routing, the handshake injection, and the same-origin write. All three have broken.
 *
 * ```bash
 * pnpm --filter @summo/web build
 * cargo build --release -p summo-cli --features bundled
 * node e2e/first-run.mjs ../../target/release/summo
 * ```
 */
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const binary = resolve(process.argv[2] ?? "../../target/release/summo");
const port = Number(process.argv[3] ?? 8791);

if (!existsSync(binary)) {
  console.error(`no binary at ${binary} — build it with --features bundled`);
  process.exit(1);
}

// A directory that has never seen Summo. This is the whole point of the suite.
const home = mkdtempSync(join(tmpdir(), "summo-first-run-"));
const problems = [];
const fail = (why) => problems.push(why);

const daemon = spawn(binary, ["serve", "--port", String(port), "--no-open"], {
  env: { ...process.env, SUMMO_HOME: home },
  stdio: ["ignore", "pipe", "pipe"],
});

let output = "";
daemon.stdout.on("data", (chunk) => (output += chunk));
daemon.stderr.on("data", (chunk) => (output += chunk));

let stopped = false;
const stop = () => {
  if (stopped) return;
  stopped = true;
  daemon.kill("SIGTERM");
  rmSync(home, { recursive: true, force: true });
};
// However this ends. Every `process.exit` below, and every exception that reaches the top, used to
// leave a daemon running on a vault under `/tmp` that nothing would ever delete — the same leak
// `daemon.mjs` had, in the one suite that does not use it.
process.once("exit", stop);

/** Wait for the port rather than sleeping: a fixed sleep is either flaky or slow. */
async function ready(url, attempts = 60) {
  for (let i = 0; i < attempts; i += 1) {
    try {
      const response = await fetch(`${url}health`);
      if (response.ok) return true;
    } catch {
      // Not up yet.
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  return false;
}

const url = `http://127.0.0.1:${port}/`;
if (!(await ready(url))) {
  console.error(`daemon did not start:\n${output}`);
  stop();
  process.exit(1);
}

if (!output.includes(String(port))) fail("the daemon did not print the address to open");

const browser = await chromium.launch();
// No locale pinned: a fresh browser here is an English one, which is what most first-time users
// arrive with and what the default is for.
const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
const page = await context.newPage();
page.on("pageerror", (e) => fail(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") fail(`console: ${m.text()}`);
});

await page.goto(url, { waitUntil: "networkidle" });
await page.waitForTimeout(1200);

// The handshake reaches the app without ever appearing in a URL.
if (page.url().includes("token=")) fail("the token is in the address bar");
const injected = await page.evaluate(() => {
  const h = globalThis.__SUMMO__;
  return h && typeof h.port === "number" && typeof h.token === "string";
});
if (!injected) fail("the handshake was not injected into the page");

if ((await page.evaluate(() => document.documentElement.lang)) !== "en") {
  fail("a fresh English browser did not get English");
}

// A brand new vault leads with setup, and setup says what it needs before anything else.
const heading = await page.locator("h1").first().innerText();
if (!/welcome/i.test(heading)) fail(`expected a welcome screen, got ${JSON.stringify(heading)}`);

const setupText = await page.locator("main").innerText();
if (!/never leaves/i.test(setupText)) fail("setup did not say the audio never leaves the machine");

// Setup has to say where recognition stands, and there are two honest answers — but only one of
// them is right for the binary under test, and this used to accept either.
//
// That indifference hid a shipped bug for as long as the check existed. The daemon computed
// whether the build can transcribe and never put the field in its JSON reply, so the screen took
// the missing field for `false` and told *every* user — including everyone running a release
// binary compiled with recognition — that this build cannot transcribe. Both binaries produced the
// apology, and a check that accepts both answers could not tell them apart.
//
// So the daemon is asked directly, and the screen is held to what it said.
const handshake = await page.evaluate(() => globalThis.__SUMMO__);
const status = await fetch(`${url}onboarding`, {
  headers: { authorization: `Bearer ${handshake.token}` },
}).then((r) => r.json());

if (typeof status.recognition !== "boolean") {
  fail("/onboarding did not say whether this build can transcribe");
}

const noRecognition = /cannot recognise speech/i.test(setupText);
const offersModel = /speech model/i.test(setupText);

if (status.recognition && !offersModel) {
  fail("this build can transcribe, and setup did not offer a model");
}
if (status.recognition === false && !noRecognition) {
  fail("this build cannot transcribe, and setup did not say so");
}
console.log(
  `setup: ${status.recognition ? "offers a model" : "no recognition in this build, and it says so"}`,
);
console.log(`setup: ${setupText.split("\n").slice(0, 2).join(" · ")}`);

await page.screenshot({ path: "/tmp/shots/first-run-setup.png", fullPage: true });

// Skipping has to get you into the app, and has to stick.
await page.getByRole("button", { name: "Later" }).click();
await page.waitForTimeout(600);

const nav = page.getByRole("navigation", { name: "Screens" });
if ((await nav.count()) === 0) fail("skipping setup did not reach the app");

// A model is genuinely missing, so the banner has to say so rather than the app pretending.
const shell = await page.locator("body").innerText();
if (!/speech model/i.test(shell)) fail("nothing warned that recognition is unavailable");

// The write path. A browser sends `Origin` on every POST, even same-origin, and refusing it means
// an app that reads and never writes — which is exactly what shipped once.
await nav.getByRole("button", { name: "Notes" }).click();
await page.waitForTimeout(500);
// `exact`, because the sidebar now offers "New page" as well: a page and a note are the same
// thing, and the tree can make one too. Without it this matches both and Playwright refuses.
//
// Scoped to the empty state on top of that, which is the second time this line has had to be
// narrowed. On a vault with nothing in it there are two "New" buttons — the toolbar's and the one
// the empty state draws — and this suite is the only one that ever sees an empty vault, so it is
// the only one that hit it. The empty state's is also the one a real first-time user presses: it
// is the larger target, in the middle of the screen, under a drawing that says there is nothing
// here yet.
await page.getByRole("status").getByRole("button", { name: "New", exact: true }).click();
await page.waitForTimeout(700);

// Waited for rather than slept at. The editor is a lazily-fetched chunk, so on a cold cache it is
// not there 700 ms after the click — which is how this read as "could not create a note" on a note
// that had been created perfectly well.
const box = page.getByRole("textbox", { name: "Note body" });
await box.waitFor({ timeout: 20000 }).catch(() => undefined);
if ((await box.count()) === 0) {
  fail("could not create a note on a clean install");
} else {
  // Typed, not filled. The editor is a rich one now: `fill` writes into the DOM behind
  // ProseMirror's back, so the document never changes, nothing is dirty and nothing autosaves —
  // the test would pass on an editor that saves nothing.
  await box.click();
  await page.keyboard.type("First run");
  await page.keyboard.press("Enter");
  await page.keyboard.type("It works.");
  await page.keyboard.press("Enter");
  await page.keyboard.type("- [ ] @me Try recording");
  // Longer than the two-second autosave, so this tests the debounce rather than racing it.
  await page.waitForTimeout(2800);

  if (!/saved/i.test(await page.locator("main").innerText())) fail("the note never saved");

  // The claim that a note is a meeting nobody recorded: a task typed into one reaches the board.
  await nav.getByRole("button", { name: "Tasks" }).click();
  await page.waitForTimeout(800);
  if (!(await page.locator("main").innerText()).includes("Try recording")) {
    fail("a task written in a note did not reach the board");
  }
}

// Setup must not greet a user who already has work in the vault.
await page.reload({ waitUntil: "networkidle" });
await page.waitForTimeout(1200);
if (/welcome/i.test(await page.locator("h1").first().innerText())) {
  fail("the welcome screen came back after the vault had a note in it");
}

await page.screenshot({ path: "/tmp/shots/first-run-notes.png", fullPage: true });
await browser.close();
stop();

if (problems.length > 0) {
  console.error(`\nPROBLEMS:\n  ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log("\nfirst run ok");
