/**
 * Two vaults, one folder, and everything that has to be true before somebody trusts it with a year
 * of meetings.
 *
 * `summo-sync` has had ninety-four unit tests since it landed and one caller: a subcommand. The
 * product page advertised "encrypted sync between your machines through any shared folder" the
 * whole time, and from inside the app there was no folder to choose and no button to press. This
 * suite drives the door that was missing — the screen and the route behind it — and it drives it
 * twice, because one machine syncing is not sync.
 *
 * What it asserts, in the order somebody would find out:
 *
 * **A folder that is not there is refused before a passphrase is asked for.** Typing a secret into
 * a screen that was going to say no anyway is the one interaction order that cannot be taken back.
 *
 * **A plan writes nothing.** The first sync of an existing vault moves every file, so "see what
 * would happen" has to be true — if it wrote, nobody would ever press it a second time.
 *
 * **A wrong passphrase says so.** To the cipher, a wrong key and tampered data are one event.
 * Reporting it as corruption sends somebody looking for a broken NAS.
 *
 * **The passphrase is not written down anywhere.** Not in `settings.json`, which is plaintext, is
 * backed up, is *itself* syncable, and is the first file anybody pastes into a support thread.
 * That one is checked on disk rather than taken on trust.
 */
import { mkdtempSync, readFileSync, writeFileSync, existsSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";

const problems = [];
const shared = mkdtempSync(join(tmpdir(), "summo-sync-folder-"));
const PASSPHRASE = "mở cửa ra vừng ơi";

const here = await boot(process.argv, { name: "sync-here" });
const there = await boot(process.argv, { name: "sync-there" });

const ask = async (engine, path, body) => {
  const response = await fetch(`${engine.url}${path}?token=${engine.token}`, {
    method: body ? "POST" : "GET",
    headers: body ? { "content-type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  let parsed = null;
  try {
    parsed = JSON.parse(text);
  } catch {
    // A refusal is sometimes plain text; the caller wants the words either way.
  }
  return { ok: response.ok, status: response.status, body: parsed, text };
};

try {
  // ---- a folder that is not there ----------------------------------------
  //
  // Refused on the *settings* call, which is the one that happens before anybody has typed a
  // passphrase. A screen that accepts a bad folder and only complains at sync time has collected a
  // secret it never needed.
  const nowhere = await ask(here, "/settings/sync", {
    folder: "/definitely/not/a/folder/anywhere",
    machine: "here",
  });
  if (nowhere.ok) problems.push("a folder that does not exist was accepted");
  if (!/không thấy/.test(nowhere.text)) {
    problems.push(`a missing folder was refused without saying so: ${nowhere.text}`);
  }

  const relative = await ask(here, "/settings/sync", { folder: "somewhere/near", machine: "here" });
  if (relative.ok) problems.push("a relative folder was accepted");

  // ---- setting it up ------------------------------------------------------
  for (const [engine, name] of [
    [here, "máy bàn"],
    [there, "laptop"],
  ]) {
    const saved = await ask(engine, "/settings/sync", { folder: shared, machine: name });
    if (!saved.ok) problems.push(`${name}: the folder was refused: ${saved.text}`);
    const state = await ask(engine, "/sync");
    if (state.body?.folder !== shared) {
      problems.push(`${name}: the folder did not stick: ${state.text}`);
    }
    if (state.body?.machine !== name) {
      problems.push(`${name}: the machine name did not stick: ${state.text}`);
    }
    // Nothing has synced yet, and the screen leans on this to warn that the first run uploads
    // everything. A wrong answer here is a surprise rather than a decision.
    if (state.body?.synced_before !== false) {
      problems.push(`${name}: claims to have synced before, on a fresh vault: ${state.text}`);
    }
    if (state.body?.problem !== null) {
      problems.push(`${name}: reports a problem with a folder that is fine: ${state.text}`);
    }
  }

  // ---- a plan writes nothing ----------------------------------------------
  const before = readdirSync(shared).length;
  const planned = await ask(here, "/sync", { passphrase: PASSPHRASE, dry_run: true });
  if (!planned.ok) problems.push(`the plan failed: ${planned.text}`);
  if (planned.body?.applied !== false) problems.push("a dry run reported itself as applied");
  if (!(planned.body?.summary?.uploaded > 0)) {
    problems.push(`a fresh vault planned no uploads: ${planned.text}`);
  }
  if (!(planned.body?.steps?.length > 0)) {
    problems.push("the plan listed no files, so the screen would show a count and nothing else");
  }
  // The salt is created on first contact and is not a secret, and `blobs/` is the folder's own
  // shape rather than anything in it. What must not appear is a blob or a manifest.
  const blobs = existsSync(join(shared, "blobs")) ? readdirSync(join(shared, "blobs")) : [];
  if (blobs.length > 0) problems.push(`a dry run wrote ${blobs.length} blob(s)`);
  const wrote = readdirSync(shared).filter((name) => /manifest/.test(name));
  if (wrote.length > 0) problems.push(`a dry run wrote: ${wrote.join(", ")}`);
  console.log(`plan: ${planned.body.steps.length} file(s), nothing written`);

  // ---- a real run, both ways ----------------------------------------------
  const pushed = await ask(here, "/sync", { passphrase: PASSPHRASE, dry_run: false });
  if (!pushed.ok) problems.push(`the sync failed: ${pushed.text}`);
  if (pushed.body?.applied !== true) problems.push("a real run did not report itself as applied");
  if (readdirSync(shared).length <= before) problems.push("a real run wrote nothing");

  const pulled = await ask(there, "/sync", { passphrase: PASSPHRASE, dry_run: false });
  if (!pulled.ok) problems.push(`the second machine failed: ${pulled.text}`);

  // The meeting the seeder writes, arriving on the other machine. This is the whole feature: not
  // that a file moved, but that a meeting somebody recorded on one laptop opens on another.
  const arrived = readdirSync(join(there.home, "vault/meetings"), { recursive: true }).filter(
    (name) => String(name).endsWith(".md"),
  );
  if (arrived.length === 0) {
    problems.push("nothing arrived in the second vault");
  } else {
    console.log(`arrived: ${arrived.length} file(s) in the second vault`);
  }

  // And the daemon now knows it has synced, which is what stops the screen warning about a first
  // run forever.
  const settled = await ask(here, "/sync");
  if (settled.body?.synced_before !== true) {
    problems.push(`after a sync the daemon still says it has never synced: ${settled.text}`);
  }

  // ---- and nothing has been lined up for deletion --------------------------
  //
  // The check this suite was written and immediately earned its keep on. Both vaults are seeded
  // identically, so the second machine's run correctly had nothing to do — and a run with nothing
  // to do used to publish an empty manifest over the one the first machine had just written. The
  // first machine's *next* run then read an empty remote against a full base, called it a
  // deletion, and planned to remove every file in the vault.
  //
  // A plan rather than a run, deliberately: asserting on the plan says what *would* happen without
  // this suite having to be the thing that loses the data if it regresses.
  const after = await ask(here, "/sync", { passphrase: PASSPHRASE, dry_run: true });
  const deleting = (after.body?.steps ?? []).filter((step) =>
    String(step.action).startsWith("delete"),
  );
  if (deleting.length > 0) {
    problems.push(
      `after a clean two-way sync, the next run would delete ${deleting.length} file(s): ` +
        deleting.map((step) => `${step.action} ${step.path}`).join(", "),
    );
  }
  const quiet = Object.values(after.body?.summary ?? {}).every((count) => count === 0);
  if (!quiet) {
    problems.push(`two settled vaults still plan work: ${JSON.stringify(after.body?.summary)}`);
  }

  // ---- a wrong passphrase --------------------------------------------------
  const wrong = await ask(there, "/sync", { passphrase: "sai bét", dry_run: true });
  if (wrong.ok) problems.push("a wrong passphrase was accepted");
  if (!/passphrase/i.test(wrong.text)) {
    problems.push(`a wrong passphrase was reported as something else: ${wrong.text}`);
  }

  // ---- the passphrase is nowhere on disk -----------------------------------
  //
  // The sharpest check in this file. `settings.json` is plaintext, ends up in backups, is itself
  // synced, and is the first file anybody pastes into a support thread — and the obvious place to
  // put a passphrase is one line under the folder path it belongs to.
  for (const [engine, name] of [
    [here, "here"],
    [there, "there"],
  ]) {
    const file = join(engine.home, "settings.json");
    const text = existsSync(file) ? readFileSync(file, "utf8") : "";
    if (text.includes(PASSPHRASE)) {
      problems.push(`${name}: the sync passphrase is written into settings.json`);
    }
    if (/passphrase|password|secret/i.test(text)) {
      problems.push(`${name}: settings.json gained a secret-shaped field: ${text.slice(0, 300)}`);
    }
  }

  // ---- and the screen ------------------------------------------------------
  const browser = await chromium.launch();
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 900 },
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && problems.push(`console: ${m.text()}`));

  await page.goto(`${here.url}?port=${here.port}&token=${here.token}#/settings?section=sync`, {
    waitUntil: "networkidle",
  });
  await page.getByTestId("settings-sync").waitFor({ timeout: 10_000 });

  // The folder the daemon knows about, on screen. A panel that opens empty on a configured vault
  // reads as "sync is off" and is the reason somebody sets it up twice.
  const shownFolder = await page.getByTestId("sync-folder").inputValue();
  if (shownFolder !== shared) {
    problems.push(`the screen shows \`${shownFolder}\` where the daemon has \`${shared}\``);
  }
  const shownMachine = await page.getByTestId("sync-machine").inputValue();
  if (shownMachine !== "máy bàn") {
    problems.push(`the screen shows the machine as \`${shownMachine}\``);
  }

  // Both buttons refuse to run until there is a passphrase. A sync button that is live with an
  // empty field is a request the daemon will refuse, reported as a failure the user caused.
  for (const id of ["sync-plan", "sync-run"]) {
    if (await page.getByTestId(id).isEnabled()) {
      problems.push(`${id} is live with no passphrase typed`);
    }
  }

  await page.getByTestId("sync-passphrase").fill(PASSPHRASE);
  if (!(await page.getByTestId("sync-plan").isEnabled())) {
    problems.push("the plan button stayed disabled after a passphrase was typed");
  }

  // The password field is a password field. Not decoration: this one is typed in meetings, on
  // shared screens, in cafés.
  const type = await page.getByTestId("sync-passphrase").getAttribute("type");
  if (type !== "password") problems.push(`the passphrase field is a \`${type}\` field`);

  await page.getByTestId("sync-plan").click();
  await page.getByTestId("sync-report").waitFor({ timeout: 30_000 });
  const report = await page.getByTestId("sync-report").innerText();
  if (report.trim().length < 5) problems.push("the report drew nothing");
  console.log(`screen says: ${report.split("\n")[0]}`);

  // Run it, and the field empties. A passphrase left sitting in a form control for as long as the
  // screen is open is one a screenshot or a shoulder picks up.
  await page.getByTestId("sync-run").click();
  await page.waitForTimeout(2_000);
  const left = await page.getByTestId("sync-passphrase").inputValue();
  if (left !== "") problems.push("the passphrase stayed in the field after the run");

  // Nothing typed into this screen reaches the browser's own storage either.
  const stored = await page.evaluate(() => JSON.stringify(window.localStorage));
  if (stored.includes(PASSPHRASE)) problems.push("the passphrase is in localStorage");

  await browser.close();
} finally {
  await here.stop();
  await there.stop();
}

// A folder left behind on a failed run is worth keeping; on a clean one it is litter. Written
// rather than deleted, so a reader who wants to look has something to look at.
writeFileSync(join(shared, ".summo-e2e"), "left by e2e/sync.mjs\n");

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const problem of problems) console.error(`  ✗ ${problem}`);
  process.exit(1);
}
console.log("\nsync ok: two vaults through one folder, and the passphrase is nowhere on disk");
