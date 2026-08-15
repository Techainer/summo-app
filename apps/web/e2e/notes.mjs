/**
 * Starting a note that is already the right shape.
 *
 * A blank page is the right default and a poor only option: people were typing the same four sets
 * of headings by hand — an idea, a decision, a list of things to do, a day's journal — and a note
 * app that watched them do it and offered nothing is the one they stop opening.
 *
 * What this asserts is the part that would break silently: that choosing a kind puts its headings
 * *in the file*, rather than into some hidden template state that a later edit or an export would
 * lose.
 */
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const problems = [];
const engine = await daemon(process.argv, { name: "notes" });
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 950 },
});
const page = await context.newPage();
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/notes`, {
  waitUntil: "networkidle",
});

// ---- the kinds on offer ---------------------------------------------------
// The New button itself makes a blank note; the caret beside it offers the shapes.
await page.getByRole("button", { name: "Loại ghi chú" }).click();
const menu = page.getByTestId("note-kinds");
await menu.waitFor({ timeout: 10000 }).catch(() => problems.push("no kinds were offered"));

const offered = (await menu.innerText()).split("\n").filter(Boolean);
if (offered.length < 5)
  problems.push(`only ${offered.length} kinds offered: ${offered.join(", ")}`);
if (!/Trống/.test(offered[0] ?? "")) {
  problems.push(`a blank note should be first, got "${offered[0]}"`);
}

// ---- a decision note starts with a decision's headings --------------------
await menu.getByRole("button", { name: "Quyết định" }).click();

const body = page.getByLabel("Nội dung ghi chú");
await body.waitFor({ timeout: 10000 });
// The seed has to be in the editor, which is what proves it is in the file rather than a label.
// Read as rendered text: the editor is a document now, not a textarea, and a `## Bối cảnh` that
// arrived as a heading is a `## Bối cảnh` that will be written back as one.
await page
  .waitForFunction(
    () => [...document.querySelectorAll(".tiptap h2")].some((h) => h.textContent === "Bối cảnh"),
    { timeout: 10000 },
  )
  .catch(() => problems.push("the decision note did not start with a decision's headings"));

// ---- and it survives a reload, because it was saved -----------------------
//
// Reopened by id rather than by clicking the title. Two lists on screen hold this note — the notes
// rail and the sidebar tree — and which one a text match hits is not what is being asserted here.
const id = await (async () => {
  const listed = await (await fetch(`${engine.url}/notes?token=${engine.token}`)).json();
  const found = listed.find((note) => note.title === "Quyết định");
  if (!found) problems.push(`no decision note in ${JSON.stringify(listed.map((n) => n.title))}`);
  return found?.id;
})();

{
  await body.locator("> *").last().click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Ngọc chốt.");
  await page.waitForTimeout(3000);

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${id}`, {
    waitUntil: "networkidle",
  });
  await page
    .locator(".tiptap")
    .waitFor({ timeout: 10000 })
    .catch(() => problems.push("the note it had just been editing richly reopened as plain text"));
  const after = await page
    .locator(".tiptap")
    .innerText()
    .catch(() => "");
  if (!after.includes("Ngọc chốt.")) problems.push(`the edit was not saved: ${after}`);
  if (!after.includes("Bối cảnh")) problems.push(`the seed lost its headings: ${after}`);
}

// ---- `/` inserts a block, and a to-do is a task ---------------------------
//
// The point of the checkbox is not the checkbox. `- [ ] …` is the line `summo_vault::tasks`
// already parses, so a to-do typed in a note is on the task board without anything syncing.
{
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Việc tuần này", body: "" }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  const editor = page.locator(".tiptap");
  await editor.waitFor({ timeout: 10000 });
  // An empty note, so the caret has one place to be. Driving a caret to the end of a document is a
  // fight with the browser that says nothing about the feature under test — `Control+End` does not
  // reliably reach it, and a `/` typed mid-word is correctly not a request for the block menu.
  await page.waitForFunction(() => (document.querySelector(".tiptap")?.textContent ?? "x") === "", {
    timeout: 10000,
  });
  await editor.click();
  await page.keyboard.type("/");

  const blocks = page.getByTestId("block-menu");
  await blocks.waitFor({ timeout: 5000 }).catch(() => problems.push("`/` opened no block menu"));

  if ((await blocks.count()) > 0) {
    await blocks.getByRole("button", { name: "Việc cần làm" }).click();
    // The menu leaves when the block has been applied, and applying it deletes the `/` that asked
    // for it. Typing into the gap between those two costs the first few characters — which is what
    // CI, being slower than this machine, found.
    await blocks.waitFor({ state: "detached", timeout: 5000 });
    await page.waitForTimeout(300);
    await page.keyboard.type("@ngoc Chốt giá");
    await page.waitForTimeout(3500);

    const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
    if (!doc.text.includes("- [ ] @ngoc Chốt giá")) {
      problems.push(`a to-do did not reach the file as a task line: ${JSON.stringify(doc.text)}`);
    }

    const tasks = await (await fetch(`${engine.url}/tasks?token=${engine.token}`)).json();
    if (!JSON.stringify(tasks).includes("Chốt giá")) {
      problems.push("a to-do typed in a note never reached the task board");
    }
  }
}

// ---- the block menu narrows, and the keyboard drives it -------------------
//
// A menu of ten that cannot be narrowed is a menu you read every time, and one that only answers
// to a mouse is one a person typing never uses. `/vi` finding `Việc cần làm` also has to work from
// a keyboard with no Vietnamese layout, which is the normal case here.
{
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Bàn phím", body: "" }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  const editor = page.locator(".tiptap");
  await editor.waitFor({ timeout: 10000 });
  await page.waitForFunction(() => (document.querySelector(".tiptap")?.textContent ?? "x") === "", {
    timeout: 10000,
  });
  await editor.click();

  const menu = page.getByTestId("block-menu");

  await page.keyboard.type("/vi");
  await page.waitForTimeout(400);
  const narrowed = await menu.innerText().catch(() => "");
  if (narrowed.trim() !== "Việc cần làm") {
    problems.push(`\`/vi\` narrowed to ${JSON.stringify(narrowed)}`);
  }

  // Enter picks it. Enter used to reach ProseMirror first and split the paragraph instead, leaving
  // `/vi` in the document as text — the menu's `preventDefault` arrived after the editor's own
  // handler, because at the target both phases fire in registration order.
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
  await page.keyboard.type("@ngoc Từ bàn phím");
  await page.waitForTimeout(3500);
  const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
  if (doc.text.trim() !== "- [ ] @ngoc Từ bàn phím") {
    problems.push(`the keyboard did not insert a to-do: ${JSON.stringify(doc.text)}`);
  }
}

// ---- and a query that names nothing is text ------------------------------
{
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Không khớp", body: "" }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  await page.locator(".tiptap").waitFor({ timeout: 10000 });
  await page.waitForFunction(() => (document.querySelector(".tiptap")?.textContent ?? "x") === "", {
    timeout: 10000,
  });
  await page.locator(".tiptap").click();

  await page.keyboard.type("/etc/passwd");
  await page.waitForTimeout(500);
  if ((await page.getByTestId("block-menu").count()) > 0) {
    problems.push("a path typed into a note left a block menu stuck open");
  }

  // Arrow keys move the highlight, and Escape puts the menu away without eating the `/`.
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Backspace");
  await page.keyboard.type("/");
  await page.waitForTimeout(400);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  const chosen = await page
    .getByTestId("block-menu")
    .locator('[aria-selected="true"]')
    .innerText()
    .catch(() => "");
  if (chosen.trim() !== "Tiêu đề vừa") {
    problems.push(`two presses of ArrowDown landed on ${JSON.stringify(chosen)}`);
  }
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  if ((await page.getByTestId("block-menu").count()) > 0)
    problems.push("Escape left the menu open");
}

// ---- formatting where the text is ----------------------------------------
{
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Bôi đen", body: "chọn tôi" }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  await page.locator(".tiptap").waitFor({ timeout: 10000 });
  await page.waitForFunction(
    () => document.querySelector(".tiptap")?.textContent?.includes("chọn tôi") ?? false,
    { timeout: 10000 },
  );
  await page.locator(".tiptap").click();
  await page.keyboard.press("ControlOrMeta+a");
  await page.waitForTimeout(600);

  const bold = page.getByRole("button", { name: "Đậm" });
  if ((await bold.count()) === 0) {
    problems.push("selecting text offered no formatting");
  } else {
    await bold.click();
    await page.waitForTimeout(3500);
    const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
    if (!doc.text.includes("**chọn tôi**")) {
      problems.push(`bold did not reach the file: ${JSON.stringify(doc.text)}`);
    }
  }
}

// ---- a note keeps what the editor cannot format ---------------------------
//
// The guarantee. A converter that did its best with a table would eat the table, and the note
// would still look like a note — which is why the editor is only offered for a document that
// survives being written back, and why what is written back is what came off disk.
{
  const table = "| gói | giá |\n|---|---|\n| pro | 4 |";
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Bảng giá", body: table }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  await page.waitForTimeout(2500);

  const shown = await page.locator("main").innerText();
  if (!shown.includes("| pro | 4 |")) problems.push(`the table is not on screen: ${shown}`);

  // Touch it, so the autosave writes the whole document back, and check the table came through.
  const editable = page.locator("main .tiptap, main textarea").first();
  await editable.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" x");
  await page.keyboard.press("Backspace");
  await page.keyboard.type("x");
  await page.waitForTimeout(3500);

  const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
  if (!doc.text.includes("| pro | 4 |")) {
    problems.push(`saving ate the table: ${JSON.stringify(doc.text)}`);
  }
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("notes ok");
