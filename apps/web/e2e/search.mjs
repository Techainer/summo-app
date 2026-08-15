/**
 * ⌘K, and the two things it has to get right.
 *
 * **It has to find a note.** The vault search has always covered typed notes as well as recordings
 * — one index over both — but nothing in the interface exercised that, and the one place that did
 * cite a note sent the reader to `/meetings/<id>`, a page that does not exist. A citation whose
 * only job is letting somebody check a claim must not be the control that breaks.
 *
 * **It has to work without a Vietnamese keyboard.** Typing `mo hinh` finds `Mô hình`. A Vietnamese
 * speaker searching their own notes on a laptop with no Vietnamese layout is the normal case, and
 * an exact-match palette is one they abandon on the first try.
 */
import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { boot } from "./daemon.mjs";

const problems = [];
const fail = (message) => problems.push(message);

const engine = await boot({ name: "search" });

// A note with a word that appears nowhere else, so a hit can only have come from the note.
const notes = join(engine.home, "vault/notes");
mkdirSync(notes, { recursive: true });
writeFileSync(
  join(notes, "khinh-khi-cau.md"),
  "---\nid: 01NOTE\ndate: 2026-08-12\n---\n# Ý tưởng ra mắt\n\nThuê một chiếc khinhkhicau màu cam.\n",
);

const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 860 },
  colorScheme: "dark",
});
const page = await context.newPage();

try {
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/`, {
    waitUntil: "networkidle",
  });
  await page.locator('[data-testid="home"]').waitFor({ timeout: 10000 });

  // From anywhere, without a mouse.
  await page.keyboard.press("ControlOrMeta+k");
  const palette = page.locator('[data-testid="palette"]');
  await palette.waitFor({ timeout: 5000 });

  // Without diacritics. This is what a keyboard with no Vietnamese layout produces.
  await page.keyboard.type("mo hinh");
  await page.waitForTimeout(300);
  if (!(await palette.innerText()).includes("Mô hình")) {
    fail("`mo hinh` does not find `Mô hình` — the palette needs the tone marks typed");
  }

  // Enter goes there.
  await page.keyboard.press("Enter");
  await page.locator('[data-testid="models"]').waitFor({ timeout: 10000 });

  // And it finds what is in a note, not only what was said in a meeting.
  await page.keyboard.press("ControlOrMeta+k");
  await palette.waitFor({ timeout: 5000 });
  await page.keyboard.type("khinhkhicau");
  await page.waitForTimeout(600);
  const found = await palette.innerText();
  if (!found.includes("Ý tưởng ra mắt")) {
    fail(`a word that exists only in a note was not found: ${JSON.stringify(found)}`);
  }

  await page.screenshot({ path: "/tmp/shots/palette.png" });

  // And opening it opens the note.
  //
  // This is the half that was broken. The palette found the note and then sent the reader to
  // `/notes` with nothing open — a search result that locates the thing and loses it on the way.
  // Both kinds now live at `/pages/<id>`, so nothing pointing at a document has to know what kind
  // it is first.
  await page.keyboard.press("Enter");
  await page.waitForTimeout(1500);
  if (!/#\/pages\/01NOTE/.test(page.url())) {
    fail(`opening a note from the palette went to ${page.url()}`);
  }
  // `inputValue`, not `innerText`: a textarea's contents are its value, and `innerText` reports the
  // empty string for one however much is typed in it.
  const opened = await page.locator("main textarea").inputValue();
  if (!opened.includes("khinhkhicau")) {
    fail(`the note opened without its text: ${JSON.stringify(opened.slice(0, 200))}`);
  }

  // Escape closes it, and closing it is not a navigation.
  await page.keyboard.press("ControlOrMeta+k");
  await palette.waitFor({ timeout: 5000 });
  const before = page.url();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  if ((await palette.count()) !== 0) fail("escape does not close the palette");
  if (page.url() !== before) fail("closing the palette navigated somewhere");
} finally {
  await browser.close();
  engine.stop();
}

if (problems.length) {
  for (const problem of problems) console.error(`FAIL ${problem}`);
  process.exit(1);
}
console.log("search ok");
