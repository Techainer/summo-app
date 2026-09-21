/**
 * The summary button, pressed.
 *
 * The one control on the meeting page that reaches a language model, and no test had ever pressed
 * it. `draft.mjs` writes `<!-- summo:draft -->` into the file by hand and checks the screen around
 * it — which tests the tinting, the confirm gesture and the refine affordance, and cannot tell you
 * whether pressing the button produces anything at all.
 *
 * So the gap was the whole first half of the feature: the click, the request the daemon makes, the
 * reply becoming sections, those sections landing in the note marked, and the screen redrawing to
 * show them. `crates/summo-engine/tests/summarize.rs` covers that path in Rust; this is the part
 * only a browser can see.
 *
 * The model is a local HTTP server rather than an intercepted route, because the request is the
 * *daemon's*. `page.route` never sees it.
 */
import { chromium } from "playwright";

import { RECENT, boot } from "./daemon.mjs";
import { model, useModel } from "./llm.mjs";

const problems = [];
const fail = (message) => problems.push(message);

/** What a cooperative model answers: the standard template's headings, verbatim. */
const REPLY = `## Tóm tắt
Chốt giữ ngân sách quý bốn ở mức quý ba.

## Việc cần làm
- [ ] @Ngọc — rà lại hợp đồng máy chủ — thứ sáu
`;

const llm = await model(REPLY);
const engine = await boot({ name: "summary" });
// After boot, not before: the daemon reads its settings on every request, and writing them here
// keeps the fixture in one place rather than threading an option through `boot`.
useModel(engine.home, llm.url);

const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1300, height: 950 },
  colorScheme: "dark",
});
const page = await context.newPage();
page.on("pageerror", (e) => fail(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") fail(`console: ${m.text()}`);
});

try {
  // A meeting written for this suite rather than the seeded one.
  //
  // Two reasons. The seeded meeting already has a summary, which is the state this button is not
  // offered in — and trimming it leaves 399 characters of transcript, under the threshold that
  // stops a ten-second recording costing a request. So the fixture is replaced with a meeting long
  // enough to be worth summarising, which is what a user presses this button on.
  const note = `${engine.home}/vault/meetings/${RECENT}-hop-dau-tuan.md`;
  const { readFileSync, writeFileSync } = await import("node:fs");
  writeFileSync(
    note,
    `---
id: 01E2E0
date: ${RECENT}T10:00:00+07:00
duration: 2538
participants: ["[[Bạn]]", "[[Ngọc]]"]
tags: [weekly]
---

# Họp đầu tuần

## Transcript
**[00:01:03] Bạn** — Hôm nay mình chốt ngân sách quý bốn cho đội kỹ thuật, có mấy khoản cần xem lại. <!-- seq:0 end:83.0 -->
**[00:02:03] Ngọc** — Em đề xuất giữ nguyên mức quý ba, khoảng hai tỷ tư, vì đội chưa tuyển thêm ai. <!-- seq:1 end:143.0 -->
**[00:03:03] Bạn** — Phần hạ tầng đang vượt, tháng trước đội lên ba trăm triệu so với dự toán ban đầu. <!-- seq:2 end:203.0 -->
**[00:04:03] Ngọc** — Em sẽ rà lại hợp đồng máy chủ trước thứ sáu và gửi lại con số cho cả nhóm xem. <!-- seq:3 end:263.0 -->
**[00:05:03] Bạn** — Được, chốt vậy đi. Bình lo phần báo giá của nhà cung cấp mới, xong trong tuần này. <!-- seq:4 end:323.0 -->
**[00:06:03] Ngọc** — Nếu rẻ hơn hai mươi phần trăm thì mình chuyển, không thì giữ nguyên nhà cũ. <!-- seq:5 end:383.0 -->
`,
  );

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/01E2E0`, {
    waitUntil: "networkidle",
  });
  await page.waitForTimeout(800);

  const button = page.getByRole("button", { name: "Tóm tắt ngay", exact: true });
  if ((await button.count()) === 0) {
    fail("a meeting with no summary offers no way to make one");
  } else {
    await button.click();

    // Generous: this is a round trip through the daemon, and what is being measured is whether it
    // happens at all.
    await page.getByText("Chốt giữ ngân sách quý bốn").first().waitFor({ timeout: 30000 });
    console.log("the button produced a summary on screen");

    // The request the daemon made. A draft on screen proves something answered; only the body
    // proves the meeting was in the question — a prompt built from an empty transcript comes back
    // looking exactly like this one.
    if (llm.asked.length === 0) {
      fail("a summary appeared without the daemon asking anything");
    } else {
      const asked = llm.asked[0];
      if (!asked.includes("ngân sách")) {
        fail("the transcript did not reach the model");
      }
      if (!asked.includes("Tóm tắt")) {
        fail("the template's headings did not reach the model");
      }
      console.log(`asked once, ${asked.length} bytes, transcript included`);
    }

    // In the note, marked. The mark is the whole safety property: unconfirmed model text sits in
    // the user's own document, and that comment is the only thing separating it from their words.
    const after = readFileSync(note, "utf8");
    if (!after.includes("summo:draft")) {
      fail("the draft went into the note without its mark");
    }
    if (!after.includes("Chốt giữ ngân sách quý bốn")) {
      fail("the summary is on screen but not in the file");
    }

    // And confirming keeps the words while dropping the mark. `draft.mjs` checks this from a
    // hand-seeded marker; doing it here means the thing confirmed is the thing the button made.
    const confirm = page.getByRole("button", { name: "Xác nhận", exact: true });
    if ((await confirm.count()) === 0) {
      fail("a generated draft cannot be confirmed from the screen");
    } else {
      await confirm.first().click();
      await page.waitForTimeout(1200);
      const done = readFileSync(note, "utf8");
      if (done.includes("summo:draft")) fail("confirming left the draft mark behind");
      if (!done.includes("Chốt giữ ngân sách quý bốn")) {
        fail("confirming lost the text it was confirming");
      }
      console.log("confirmed: the mark is gone and the words are not");
    }
  }

  await page.screenshot({ path: "/tmp/shots/summary.png", fullPage: true });
} finally {
  await browser.close();
  engine.stop();
  await llm.stop();
}

if (problems.length) {
  for (const problem of problems) console.error(`FAIL ${problem}`);
  process.exit(1);
}
console.log("summary ok");
