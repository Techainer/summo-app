/**
 * One tree, both kinds of page — and an assistant that remembers what you keep asking for.
 *
 * The model this checks is the one the app is built on and the interface kept contradicting: a
 * recording *is* a note. It has audio and a transcript attached, and everything else about it — how
 * it is filed, searched, titled, opened — is what a typed note does. So the sidebar lists them
 * together, in the folders the user made, the way pages sit in Notion.
 *
 * And the second half: what somebody asks an agent to do is worth remembering. Ask twice and the
 * words come back as a button, so the fourth report costs a click rather than a paragraph. That
 * list is `vault/agents/HABITS.md` and deleting a line forgets it — asserted here, because a
 * memory the user cannot delete is the kind nobody wants.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { chromium } from "playwright";

import { boot } from "./daemon.mjs";

const problems = [];
const engine = await boot({ name: "pages" });
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 950 },
});
const page = await context.newPage();
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/`, {
  waitUntil: "networkidle",
});

// ---- the tree holds recordings and notes alike ----------------------------
{
  const tree = page.getByLabel("Thư mục");
  await tree.waitFor({ timeout: 10000 });

  // The seeded vault has a meeting filed under a folder and a note. Expanding the folder must show
  // the meeting as a page, not merely narrow a list somewhere else on screen.
  const folder = tree.getByRole("button", { name: "khach-hang" });
  if ((await folder.count()) === 0) {
    problems.push("the seeded folder is not in the tree");
  } else {
    await tree.getByRole("button", { name: /Mở rộng khach-hang|Thu gọn khach-hang/ }).click();
    await page.waitForTimeout(400);
    const listed = await tree.innerText();
    if (!/Demo khách hàng|Ý tưởng giá|Họp/.test(listed)) {
      problems.push(`no pages appeared under the folder: ${JSON.stringify(listed)}`);
    }
  }
}

// ---- a page in the tree opens ---------------------------------------------
{
  const tree = page.getByLabel("Thư mục");
  const anyPage = tree
    .locator("button")
    .filter({ hasText: /Họp|Ý tưởng|Demo/ })
    .first();
  if ((await anyPage.count()) === 0) {
    problems.push("no page to open");
  } else {
    await anyPage.click();
    await page.waitForTimeout(1200);
    const url = page.url();
    if (!/#\/(meetings\/|notes\?)/.test(url)) {
      problems.push(`clicking a page went nowhere useful: ${url}`);
    }
  }
}

// ---- a new page, from the tree --------------------------------------------
{
  await page.getByRole("button", { name: "Trang mới", exact: true }).click();
  await page.waitForTimeout(1500);
  if (!/#\/notes\?open=/.test(page.url())) {
    problems.push(`"new page" did not open the page it made: ${page.url()}`);
  }
}

// ---- filing a page from the tree ------------------------------------------
//
// The page just made is unfiled, which is where a new page starts. Filing it is the gesture the
// tree exists for, and it is checked through the menu rather than through the drag: dragging is
// pointer-only and undelivered on touch, so the menu is the path that has to work everywhere.
{
  const tree = page.getByLabel("Thư mục");
  const made = tree.getByRole("button", { name: /Chuyển Ghi chú mới/ }).first();
  await made
    .waitFor({ timeout: 10000 })
    .catch(() => problems.push("the new page has no way to be filed"));

  if ((await made.count()) > 0) {
    await made.click();
    const menu = page.getByTestId("move-page").first();
    await menu.waitFor({ timeout: 5000 }).catch(() => problems.push("no destinations offered"));
    await menu.getByRole("button", { name: "khach-hang" }).first().click();
    await page.waitForTimeout(1200);

    const listed = await (await fetch(`${engine.url}/library?token=${engine.token}`)).json();
    const moved = listed.groups
      .flatMap((g) => g.meetings)
      .find((m) => m.title === "Ghi chú mới");
    if (!moved) {
      problems.push("the filed page vanished from the vault");
    } else if (moved.folder !== "khach-hang") {
      problems.push(`filing put the page in ${JSON.stringify(moved.folder)}`);
    } else if (!moved.file.includes("notes/")) {
      // A typed note filed into a folder used to be carried out of `notes/` and into the
      // recordings tree, because every destination was computed from `meetings/`.
      problems.push(`filing moved the note out of notes/: ${moved.file}`);
    }
  }
}

// ---- and back out again, by dragging --------------------------------------
{
  const tree = page.getByLabel("Thư mục");
  const row = tree.locator("[draggable='true']").filter({ hasText: "Ghi chú mới" }).first();
  if ((await row.count()) === 0) {
    problems.push("the filed page is not in the tree to drag");
  } else {
    await row.dragTo(tree.getByRole("button", { name: "Tất cả", exact: true }));
    await page.waitForTimeout(1200);
    const listed = await (await fetch(`${engine.url}/library?token=${engine.token}`)).json();
    const moved = listed.groups
      .flatMap((g) => g.meetings)
      .find((m) => m.title === "Ghi chú mới");
    if (moved?.folder !== "") {
      problems.push(`dragging to the root left it in ${JSON.stringify(moved?.folder)}`);
    }
  }
}

// ---- what you keep asking for becomes a button ----------------------------
{
  // Written straight into the vault rather than by asking twice through the interface: an agent run
  // needs a language model, and this is a test about the memory, not about the model.
  // The roster is seeded on first use, so on a vault nobody has asked anything of yet the
  // directory is not there — which is exactly the state this is testing from.
  mkdirSync(join(engine.home, "vault", "agents"), { recursive: true });
  const habits = join(engine.home, "vault", "agents", "HABITS.md");
  writeFileSync(
    habits,
    "# Thói quen\n\n- 2026-08-01 — viết báo cáo sau họp\n- 2026-08-08 — viết báo cáo sau họp\n" +
      "- 2026-08-09 — chỉ nhờ một lần\n",
  );

  const response = await fetch(`${engine.url}/agent/habits?token=${engine.token}`);
  const learned = await response.json();
  if (learned.length !== 1) {
    problems.push(`expected one habit, got ${JSON.stringify(learned)}`);
  } else if (learned[0].times !== 2) {
    problems.push(`the habit was not counted: ${JSON.stringify(learned[0])}`);
  }

  // And it reaches the meeting screen, where the asking happens.
  const meeting = await (await fetch(`${engine.url}/library?token=${engine.token}`)).json();
  const first = meeting.groups.flatMap((g) => g.meetings).find((m) => m.kind === "meeting");
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/meetings/${first.id}`, {
    waitUntil: "networkidle",
  });
  const offered = page.getByTestId("ask-habits");
  await offered
    .waitFor({ timeout: 10000 })
    .catch(() => problems.push("the habit was never offered on the meeting"));
  if ((await offered.count()) > 0) {
    const text = await offered.innerText();
    if (!/viết báo cáo sau họp/.test(text)) problems.push(`wrong habit offered: ${text}`);
    if (/chỉ nhờ một lần/.test(text)) problems.push("asked once is not a habit, and was offered");
  }
}

// ---- deleting a line forgets it -------------------------------------------
{
  const habits = join(engine.home, "vault", "agents", "HABITS.md");
  const kept = readFileSync(habits, "utf8")
    .split("\n")
    .filter((line) => !line.includes("2026-08-08"))
    .join("\n");
  writeFileSync(habits, kept);

  const learned = await (await fetch(`${engine.url}/agent/habits?token=${engine.token}`)).json();
  if (learned.length !== 0) {
    problems.push(`deleting the line did not forget it: ${JSON.stringify(learned)}`);
  }
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("pages ok");
