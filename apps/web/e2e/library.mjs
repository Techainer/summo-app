/**
 * The library, driven the way a person drives it.
 *
 * Recording is covered by `full-flow.mjs`; this covers the other half — the meetings that already
 * exist. It runs against a real daemon and a real vault on disk, so it catches the things unit
 * tests structurally cannot: a route that is not registered, a field the app reads under a
 * different name than the daemon writes, a filter that returns everything.
 */
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "library" });
const { url: appUrl, port, token } = engine;

const browser = await chromium.launch();
// The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
// this the app honours the machine's locale — which is exactly what it should do, and which made
// every assertion here fail the moment translation landed.
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 820 },
  colorScheme: "dark",
});
const page = await context.newPage();

const problems = [];
page.on("console", (m) => {
  if (m.type() === "error") problems.push(`console: ${m.text()}`);
});
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

const fail = (why) => {
  problems.push(why);
};

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });
await page.getByRole("button", { name: "Kho" }).click();
await page.locator('[data-testid="meeting-row"]').first().waitFor({ timeout: 10000 });

// The dashboard is what a user sees before picking a meeting.
const tiles = await page.locator('[data-testid="tile-value"]').allInnerTexts();
console.log(`dashboard tiles: ${tiles.join(" · ")}`);
await page.screenshot({ path: "/tmp/shots/library.png" });

const headings = await page.locator('[data-testid="group-heading"]').allInnerTexts();
console.log(`day headings: ${headings.join(" | ")}`);
if (!headings.some((h) => /hôm nay|hôm qua|tháng|thứ/i.test(h))) {
  fail(`day headings did not render as dates: ${JSON.stringify(headings)}`);
}

const rows = await page.locator('[data-testid="meeting-title"]').allInnerTexts();
console.log(`meetings listed: ${rows.join(" | ")}`);
if (rows.length < 2) fail(`expected the seeded meetings, got ${rows.length}`);

// Grouping by week must not lose meetings.
// `exact`: the seeded meetings are called "Họp tuần N", so a substring match finds five buttons.
await page.getByRole("button", { name: "Tuần", exact: true }).click();
await page.waitForTimeout(300);
const weekly = await page.locator('[data-testid="meeting-title"]').count();
if (weekly !== rows.length) fail(`grouping by week changed the count: ${rows.length} → ${weekly}`);
const weekHeadings = await page.locator('[data-testid="group-heading"]').allInnerTexts();
if (!weekHeadings.some((h) => /^tuần/i.test(h)))
  fail(`week headings missing: ${JSON.stringify(weekHeadings)}`);
await page.getByRole("button", { name: "Ngày", exact: true }).click();

// Search without tone marks is the point of the fold table.
await page.getByLabel("Tìm kiếm").fill("ngan sach");
await page.locator('[data-testid="excerpt"]').first().waitFor({ timeout: 5000 });
const excerpts = await page.locator('[data-testid="excerpt"]').allInnerTexts();
console.log(`search "ngan sach" → ${excerpts.length} excerpt(s)`);
if (!excerpts.some((e) => e.includes("ngân sách"))) {
  fail(`searching without tone marks did not find the toned text: ${JSON.stringify(excerpts)}`);
}
await page.screenshot({ path: "/tmp/shots/library-search.png" });

await page.getByLabel("Tìm kiếm").fill("");
await page.waitForTimeout(300);

// Open meetings until one has a transcript, and check it arrived.
//
// Not simply the first row: a vault accumulates meetings, and several of them legitimately have no
// transcript — a note filed by hand, or a recording that captured silence. Asserting on whichever
// happens to sort first tests the fixture, not the code.
let lines = 0;
const rowCount = await page.locator('[data-testid="meeting-row"]').count();
for (let i = 0; i < rowCount && lines === 0; i += 1) {
  await page.locator('[data-testid="meeting-row"]').nth(i).click();
  await page.locator('[data-testid="meeting"]').waitFor({ timeout: 5000 });
  // The frame renders before the transcript fetch resolves; counting immediately reads zero on a
  // meeting that does have one.
  await page.waitForTimeout(400);
  lines = await page.locator('[data-testid="transcript-line"]').count();
}
console.log(`transcript lines in detail view: ${lines}`);
if (lines === 0) fail("the meeting detail showed no transcript");
await page.screenshot({ path: "/tmp/shots/library-meeting.png" });

// Rename, and confirm it survives a refetch rather than only living in React state.
const title = page.getByLabel("Tên cuộc họp");
await title.fill("Họp ngân sách quý ba");
await title.blur();
await page.waitForTimeout(600);
// Anywhere in the list, not first: the loop above selects whichever meeting has a transcript,
// and the list is ordered by date. Asserting position made this pass only for a vault whose
// newest document happened to be the one being renamed.
const titles = await page.locator('[data-testid="meeting-title"]').allInnerTexts();
if (!titles.includes("Họp ngân sách quý ba"))
  fail(`rename did not reach the list: got ${JSON.stringify(titles)}`);

// File it into a folder, which is the organisation feature this screen exists for.
await page.getByRole("textbox", { name: "Thẻ" }).fill("product, weekly");
await page.getByRole("textbox", { name: "Thẻ" }).blur();
await page.waitForTimeout(600);
const tags = await page.locator('[data-testid="finder"] button').allInnerTexts();
console.log(`tags after edit: ${tags.join(" ")}`);
if (!tags.some((t) => t.includes("product")))
  fail(`the new tag did not reach the facets: ${JSON.stringify(tags)}`);

await page.screenshot({ path: "/tmp/shots/library-edited.png" });

// Settings is only the language model, and it has to say plainly where words go.
await page.getByRole("button", { name: "Cài đặt" }).click();
await page.locator('[data-testid="settings"]').waitFor({ timeout: 5000 });
// Settings is six sections behind a rail now; the model lives in the one about the language model.
await page.getByTestId("settings-tab-ai").click();
await page.locator('[data-testid="settings-ai"]').waitFor({ timeout: 5000 });
// Exact: the settings screen has a second model field for the translation model, and a
// substring match picks up its checkbox too — which fails as an ambiguity rather than as a
// wrong field, so it is at least loud.
const model = page.getByLabel("Mô hình", { exact: true });
await model.fill("qwen3:8b");
await model.blur();
await page.waitForTimeout(400);
await page.getByRole("button", { name: /Thử kết nối/ }).click();
await page.locator('[data-testid="test-result"]').waitFor({ timeout: 15000 });
const verdict = await page.locator('[data-testid="test-result"]').innerText();
console.log(`connection test: ${verdict.split("\n")[0]}`);
if (!/không có gì rời khỏi máy/i.test(verdict)) {
  fail(`the settings screen did not say where transcript text goes: ${JSON.stringify(verdict)}`);
}
await page.screenshot({ path: "/tmp/shots/settings.png" });

await browser.close();
engine.stop();

console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nno problems");
process.exit(problems.length ? 1 : 0);
