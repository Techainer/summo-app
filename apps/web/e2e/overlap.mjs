/**
 * A meeting where two people talk over each other, drawn.
 *
 * The unit tests decide *which* lines overlap. What they cannot see is whether a reader can tell:
 * the whole failure is that two simultaneous utterances render as a plain list, read as a question
 * and an answer, and nothing on screen says otherwise. That is only checkable on screen.
 *
 * Also checks the two things a live transcript gets wrong when nobody looks at it in a second
 * language: a repeated speaker name on every line, and italicised Japanese — which a browser
 * produces by shearing glyphs, because the script has no italics.
 */
import { chromium } from "playwright";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { boot } from "./daemon.mjs";

const problems = [];
const fail = (message) => problems.push(message);

const engine = await boot({ name: "overlap" });

// Ngọc and Dũng speak at once at 00:00:18 — his line starts three seconds before hers ends.
const meetings = join(engine.home, "vault/meetings");
mkdirSync(meetings, { recursive: true });
writeFileSync(
  join(meetings, "chen-loi.md"),
  `---
id: 01OVERLAP
date: 2026-08-12
title: Họp chen lời
---

# Họp chen lời

## Transcript
**[00:00:01] Ngọc** — Chiều nay mình chốt lại spec API nhé <!-- seq:1 end:8.0 -->
**[00:00:08] Ngọc** — rồi gửi cho bên khách hàng <!-- seq:2 end:14.0 -->
**[00:00:14] Ngọc** — mình nghĩ thứ Sáu là kịp <!-- seq:3 end:22.0 -->
**[00:00:19] Dũng** — không kịp đâu, cần thêm hai ngày test tải <!-- seq:4 end:27.0 -->
**[00:00:40] Ngọc** — ok vậy dời sang tuần sau <!-- seq:5 end:45.0 -->
`,
);

const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 860 },
  colorScheme: "dark",
});
const page = await context.newPage();

try {
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/meetings/01OVERLAP`, {
    waitUntil: "networkidle",
  });
  // The meeting screen renders the stored transcript as chips; the recording screen renders the
  // live one as a list. Both go through the same reading rules, and this is the one a reader
  // comes back to.
  await page.getByRole("heading", { name: "Họp chen lời" }).waitFor({ timeout: 10000 });
  // The meeting screen opens on comments; the transcript is the other tab.
  await page.getByRole("radio", { name: "Bản ghi", exact: true }).click();
  await page.waitForTimeout(600);

  // Dũng's line began before Ngọc's ended. It has to be marked, and say so in words — a coloured
  // rule alone is invisible to anyone who cannot see the colour.
  const overlapping = page.locator("[data-overlapping]");
  if ((await overlapping.count()) !== 1) {
    fail(`expected exactly one overlapping line, found ${await overlapping.count()}`);
  } else if (!(await overlapping.innerText()).includes("cùng lúc")) {
    fail("the overlap is drawn but not named, so a screen reader announces nothing");
  }

  // Three consecutive lines from Ngọc are one person talking. Her name belongs above the first.
  const names = await page.getByRole("main").innerText();
  const ngoc = (names.match(/Ngọc/g) ?? []).length;
  if (ngoc > 2) {
    fail(`Ngọc is named ${ngoc} times for one run of speech plus a later one`);
  }

  // Twenty-six seconds of silence before her last line.
  if ((await page.locator('[data-testid="pause"]').count()) < 1) {
    fail("a long silence between two lines is not shown");
  }

  await page.screenshot({ path: "/tmp/shots/overlap-vi.png" });

  // The same meeting translated into Japanese. Italic CJK is synthesised by shearing the glyphs,
  // which on dense characters is harder to read and looks like a rendering fault.
  const sheared = await page.evaluate(() => {
    const marker = document.createElement("p");
    marker.lang = "ja";
    marker.textContent = "同時に発言";
    document.body.append(marker);
    const style = getComputedStyle(marker);
    const result = style.fontStyle;
    marker.remove();
    return result;
  });
  if (sheared !== "normal") {
    fail(`Japanese is rendered ${sheared}, which shears glyphs that have no italic form`);
  }
} finally {
  await browser.close();
  engine.stop();
}

if (problems.length) {
  for (const problem of problems) console.error(`FAIL ${problem}`);
  process.exit(1);
}
console.log("overlap ok");
