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

// ---- a table is a table now, and it is still a table in the file ---------
//
// This block used to assert the opposite: that `| gói | giá |` stayed on screen as the characters
// somebody typed, because the converter had no node for a table. It has one now, and the assertion
// that matters did not change — what is written back has to be what a Markdown reader would show.
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

  // Drawn as a table. The pipes were the old assertion and would now mean the note had fallen back
  // to the plain textarea — which is a real failure and one that looks like nothing on screen.
  const cells = await page.locator(".tiptap table td, .tiptap table th").allInnerTexts();
  if (cells.join("|") !== "gói|giá|pro|4") {
    problems.push(`the table did not render as a table: ${JSON.stringify(cells)}`);
  }

  // Editing inside a cell: the table controls appear, a row is added, and the file keeps the shape.
  await page.locator(".tiptap table td").first().click();
  const tools = page.getByTestId("table-tools");
  await tools.waitFor({ timeout: 5000 }).catch(() => problems.push("no table controls in a table"));
  if ((await tools.count()) > 0) {
    await tools.getByRole("button", { name: "Thêm hàng" }).click();
    await page.waitForTimeout(3500);

    const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
    const rows = doc.text.split("\n").filter((line) => line.startsWith("|"));
    if (rows.length !== 4) {
      problems.push(`adding a row did not reach the file: ${JSON.stringify(doc.text)}`);
    }
    if (!doc.text.includes("| pro | 4 |")) {
      problems.push(`saving ate the table: ${JSON.stringify(doc.text)}`);
    }
  }
}

// ---- and a note keeps what the editor still cannot format -----------------
//
// The guarantee, on something there is no node for. A converter that did its best with a footnote
// would eat the footnote, and the note would still look like a note — which is why the editor is
// only offered for a document that survives being written back.
{
  const footnote = "Câu có chú thích[^1]\n\n[^1]: chú thích ở đây";
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Chú thích", body: footnote }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  await page.waitForTimeout(2500);

  const editable = page.locator("main .tiptap, main textarea").first();
  await editable.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" x");
  await page.keyboard.press("Backspace");
  await page.keyboard.type("x");
  await page.waitForTimeout(3500);

  const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
  if (!doc.text.includes("[^1]: chú thích ở đây")) {
    problems.push(`saving ate the footnote: ${JSON.stringify(doc.text)}`);
  }
}

// ---- a picture is a file in the vault, not base64 in the note -------------
//
// The link in the Markdown is `attachments/<name>` — relative to the vault root, so the note opens
// in Obsidian too — and the browser is served the bytes by the daemon. Both halves are asserted,
// because a link that is right and an image that does not load is the same broken note.
{
  // A one-pixel PNG. Enough for the sniffer, which reads the format out of the bytes and never out
  // of what the client says it uploaded.
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  const stored = await (
    await fetch(`${engine.url}/attachments?token=${engine.token}`, {
      method: "POST",
      body: png,
    })
  ).json();
  if (!/^attachments\/[0-9a-f]{32}\.png$/.test(stored.link ?? "")) {
    problems.push(`an uploaded picture got a strange link: ${JSON.stringify(stored)}`);
  }

  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Có ảnh", body: `![sơ đồ](${stored.link})` }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  await page.waitForTimeout(2000);

  // Not `.tiptap img`: ProseMirror puts an empty `img.ProseMirror-separator` after an inline node
  // at the end of a block, and two matches is a strict-mode failure rather than an assertion.
  const shown = page.locator('.tiptap img[alt="sơ đồ"]');
  await shown.waitFor({ timeout: 5000 }).catch(() => problems.push("the picture did not render"));
  if ((await shown.count()) > 0) {
    // Loaded, not merely present. A `src` the daemon refuses is a broken image icon, which is
    // exactly what a token or a path this test got wrong would look like.
    const width = await shown.evaluate((img) => img.naturalWidth);
    if (width !== 1) problems.push(`the picture did not load: naturalWidth ${width}`);

    // Touch the note so it is written back, and check the *link* survived rather than the URL the
    // browser was given. A note that saved `http://127.0.0.1:54321/attachments/…` would look fine
    // until the daemon next started on a different port.
    await page.locator(".tiptap").click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type("x");
    await page.waitForTimeout(3500);
    const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
    if (!doc.text.includes(`![sơ đồ](${stored.link})`)) {
      problems.push(`the picture link did not survive a save: ${JSON.stringify(doc.text)}`);
    }
  }
}

// ---- a block can be picked up and put somewhere else ---------------------
//
// The feature is invisible until you hover, and what it changes is the *file* — so the assertion is
// the file. `.drag-handle` is the class the extension names its own element; nothing in this app may
// rename it, and a stylesheet that thought otherwise is why this test exists.
{
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Ba đoạn", body: "Một.\n\nHai.\n\nBa." }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  await page.locator(".tiptap").waitFor({ timeout: 10000 });
  await page.waitForTimeout(1500);

  await page.locator(".tiptap p").nth(1).hover();
  const handle = page.locator(".drag-handle");
  await handle
    .waitFor({ state: "visible", timeout: 5000 })
    .catch(() => problems.push("hovering a paragraph offered no handle to pick it up by"));

  const grip = await handle.boundingBox().catch(() => null);
  const onto = await page.locator(".tiptap p").nth(2).boundingBox();
  if (grip && onto) {
    await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
    await page.mouse.down();
    // Two moves: one to cross into the target block, one to settle below its midpoint. A single
    // jump is often read as a click, and a drop above the midpoint would be a no-op that looks
    // exactly like a broken feature.
    await page.mouse.move(onto.x + 40, onto.y + onto.height, { steps: 12 });
    await page.mouse.move(onto.x + 40, onto.y + onto.height - 2, { steps: 4 });
    await page.mouse.up();
    await page.waitForTimeout(3500);

    const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
    if (doc.text.trim() !== "Một.\n\nBa.\n\nHai.") {
      problems.push(`dragging a paragraph did not reorder the file: ${JSON.stringify(doc.text)}`);
    }
  }
}

// ---- a page inside a page -------------------------------------------------
//
// Two things have to be true at once, and they are stored in two places on purpose: the *link* is
// ordinary Markdown in the parent, so the note means something in any editor, and the *parent* is
// in the child's frontmatter, so the tree survives the file being renamed or moved.
{
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Dự án ACME", body: "" }),
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
  await page.keyboard.type("/trang");
  await page.waitForTimeout(500);

  const blocks = page.getByTestId("block-menu");
  if ((await blocks.count()) === 0) {
    problems.push("`/trang` offered no sub-page");
  } else {
    await blocks.getByRole("button", { name: "Trang con" }).click();
    await page.waitForTimeout(3500);

    const doc = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
    const link = /\[([^\]]+)\]\(\/pages\/([^)]+)\)/.exec(doc.text ?? "");
    if (!link) {
      problems.push(`no link to a sub-page in the parent: ${JSON.stringify(doc.text)}`);
    } else {
      const child = await (
        await fetch(`${engine.url}/notes/${link[2]}?token=${engine.token}`)
      ).json();
      if (child.frontmatter?.parent !== made.id) {
        problems.push(
          `the sub-page does not know its parent: ${JSON.stringify(child.frontmatter)}`,
        );
      }

      // And the tree draws it inside. The row for the parent gains a chevron; opening it shows the
      // child, which is the whole reason the parent is stored rather than only linked.
      await page.reload({ waitUntil: "networkidle" });
      await page.waitForTimeout(1500);
      const expand = page.getByRole("button", { name: "Mở rộng Dự án ACME" });
      if ((await expand.count()) === 0) {
        problems.push("the tree offered no way to open a page that contains one");
      } else {
        await expand.first().click();
        await page.waitForTimeout(500);
        const tree = await page.getByLabel("Thư mục").innerText();
        if (!tree.includes(link[1])) {
          problems.push(`the sub-page is not drawn inside its parent: ${JSON.stringify(tree)}`);
        }
      }

      // Taking it back out is the same page, one row up. It must not move the file.
      const before = (await (await fetch(`${engine.url}/notes?token=${engine.token}`)).json()).find(
        (note) => note.id === link[2],
      )?.file;
      await fetch(`${engine.url}/meetings/${link[2]}/parent?token=${engine.token}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ parent: null }),
      });
      const listed = await (await fetch(`${engine.url}/notes?token=${engine.token}`)).json();
      const after = listed.find((note) => note.id === link[2]);
      if (after?.file !== before) {
        problems.push(`un-nesting moved the file: ${before} -> ${after?.file}`);
      }
    }
  }
}

// ---- typing and leaving inside the debounce ------------------------------
//
// The autosave fires two seconds after the last keystroke. Everything above waits for it, which is
// why none of it caught this: the cleanup that runs when the editor goes away cleared the pending
// timer and never saved, so a sentence typed and then navigated away from within two seconds was
// gone — from the editor whose comment said, in those words, that it must not be.
//
// Two seconds is not an unusual amount of time to spend on a sentence before clicking something.
{
  const made = await (
    await fetch(`${engine.url}/notes?token=${engine.token}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: "Rời đi ngay", body: "" }),
    })
  ).json();

  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/${made.id}`, {
    waitUntil: "networkidle",
  });
  const editor = page.locator(".tiptap");
  await editor.waitFor({ timeout: 10000 });
  await editor.click();
  await page.keyboard.type("Chưa kịp lưu đã đi.");

  // Straight out, well inside the two seconds, and *inside the app* — a sidebar click, which is
  // what a person does. Not `page.goto`: a full page load fires `pagehide`, and a suite that
  // navigates that way is testing the unload listener rather than the thing that actually happens
  // when somebody clicks another screen. This distinction is not theoretical — the first version
  // of this test passed against the broken editor for exactly that reason.
  await page.waitForTimeout(200);
  await page
    .getByRole("navigation", { name: "Màn hình" })
    .getByRole("button", { name: "Đã lưu", exact: true })
    .click();
  await page.waitForTimeout(1500);

  const saved = await (await fetch(`${engine.url}/notes/${made.id}?token=${engine.token}`)).json();
  if (!(saved.text ?? "").includes("Chưa kịp lưu đã đi.")) {
    problems.push(`leaving inside the debounce lost the text: ${JSON.stringify(saved.text)}`);
  }
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("notes ok");
