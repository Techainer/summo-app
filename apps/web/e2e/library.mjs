/**
 * The library, driven the way a person drives it.
 *
 * Recording is covered by `full-flow.mjs`; this covers the other half — the meetings that already
 * exist. It runs against a real daemon and a real vault on disk, so it catches the things unit
 * tests structurally cannot: a route that is not registered, a field the app reads under a
 * different name than the daemon writes, a filter that returns everything.
 */
import { chromium } from 'playwright';

const [, , appUrl, port, token] = process.argv;

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 1280, height: 820 }, colorScheme: 'dark' });
const page = await context.newPage();

const problems = [];
page.on('console', (m) => { if (m.type() === 'error') problems.push(`console: ${m.text()}`); });
page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));

const fail = (why) => { problems.push(why); };

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: 'networkidle' });
await page.getByRole('button', { name: 'Thư viện' }).click();
await page.locator('[data-testid="meeting-list"] .row').first().waitFor({ timeout: 10000 });

// The dashboard is what a user sees before picking a meeting.
const tiles = await page.locator('.tile-value').allInnerTexts();
console.log(`dashboard tiles: ${tiles.join(' · ')}`);
await page.screenshot({ path: '/tmp/shots/library.png' });

const headings = await page.locator('.group h3').allInnerTexts();
console.log(`day headings: ${headings.join(' | ')}`);
if (!headings.some((h) => /hôm nay|hôm qua|tháng|thứ/i.test(h))) {
  fail(`day headings did not render as dates: ${JSON.stringify(headings)}`);
}

const rows = await page.locator('.row-title').allInnerTexts();
console.log(`meetings listed: ${rows.join(' | ')}`);
if (rows.length < 2) fail(`expected the seeded meetings, got ${rows.length}`);

// Grouping by week must not lose meetings.
await page.getByRole('button', { name: 'Tuần' }).click();
await page.waitForTimeout(300);
const weekly = await page.locator('.row-title').count();
if (weekly !== rows.length) fail(`grouping by week changed the count: ${rows.length} → ${weekly}`);
const weekHeadings = await page.locator('.group h3').allInnerTexts();
if (!weekHeadings.some((h) => /^tuần/i.test(h))) fail(`week headings missing: ${JSON.stringify(weekHeadings)}`);
await page.getByRole('button', { name: 'Ngày' }).click();

// Search without tone marks is the point of the fold table.
await page.getByLabel('Tìm kiếm').fill('ngan sach');
await page.locator('.excerpt').first().waitFor({ timeout: 5000 });
const excerpts = await page.locator('.excerpt').allInnerTexts();
console.log(`search "ngan sach" → ${excerpts.length} excerpt(s)`);
if (!excerpts.some((e) => e.includes('ngân sách'))) {
  fail(`searching without tone marks did not find the toned text: ${JSON.stringify(excerpts)}`);
}
await page.screenshot({ path: '/tmp/shots/library-search.png' });

await page.getByLabel('Tìm kiếm').fill('');
await page.waitForTimeout(300);

// Open one meeting and check the transcript actually arrived.
await page.locator('.row').first().click();
await page.locator('[data-testid="meeting"]').waitFor({ timeout: 5000 });
const lines = await page.locator('.lines li').count();
console.log(`transcript lines in detail view: ${lines}`);
if (lines === 0) fail('the meeting detail showed no transcript');
await page.screenshot({ path: '/tmp/shots/library-meeting.png' });

// Rename, and confirm it survives a refetch rather than only living in React state.
const title = page.getByLabel('Tên cuộc họp');
await title.fill('Họp ngân sách quý ba');
await title.blur();
await page.waitForTimeout(600);
const renamed = await page.locator('.row-title').first().innerText();
if (renamed !== 'Họp ngân sách quý ba') fail(`rename did not reach the list: got ${JSON.stringify(renamed)}`);

// File it into a folder, which is the organisation feature this screen exists for.
await page.getByRole('textbox', { name: 'Thẻ' }).fill('product, weekly');
await page.getByRole('textbox', { name: 'Thẻ' }).blur();
await page.waitForTimeout(600);
const tags = await page.locator('.facets button').allInnerTexts();
console.log(`tags after edit: ${tags.join(' ')}`);
if (!tags.some((t) => t.includes('product'))) fail(`the new tag did not reach the facets: ${JSON.stringify(tags)}`);

await page.screenshot({ path: '/tmp/shots/library-edited.png' });
await browser.close();

console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join('\n  ')}` : '\nno problems');
process.exit(problems.length ? 1 : 0);
