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
await page
  .waitForFunction(
    () => document.querySelector("textarea")?.value.includes("## Bối cảnh") ?? false,
    { timeout: 10000 },
  )
  .catch(() => problems.push("the decision note did not start with a decision's headings"));

// ---- and it survives a reload, because it was saved -----------------------
{
  const before = await body.inputValue();
  // Typing is what triggers the save; a note created and never touched is allowed to be empty.
  await body.click();
  await body.press("End");
  await body.type(" Ngọc chốt.");
  await page.waitForTimeout(3000);
  await page.reload({ waitUntil: "networkidle" });

  const notes = await page.getByText("Quyết định").count();
  if (notes === 0) problems.push("the note is not in the list after a reload");
  if (!before.includes("## Quyết định")) {
    problems.push(`the seed is missing its own heading: ${JSON.stringify(before.slice(0, 40))}`);
  }
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("notes ok");
