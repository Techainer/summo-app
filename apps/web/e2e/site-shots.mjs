/**
 * The pictures the marketing site uses, taken from the app that is actually built.
 *
 * The site's screenshots were made by hand, once, from `/tmp/shots` — which is why the setup screen
 * on the landing page was a screen that no longer exists, and why nobody could tell: nothing
 * connects the two repositories, so an interface change and the picture of it drift apart silently
 * and the only person who notices is a visitor comparing the page to the download.
 *
 * So this produces them, from a seeded vault, at the sizes the page puts them at:
 *
 * - `wide` — 1440×920, for the hero and the gallery, where an image spans the column.
 * - `feature` — 1120×760, for the four alternating rows, where the image gets half the width.
 *   Photographed narrower on purpose: a 1440-wide screen shown 640 wide is a screenshot whose text
 *   is four pixels tall, which is what the feature rows have been showing. The app has a real
 *   layout at this width, and at this width it can be read.
 * - `phone` — 390×844, the app's own narrow layout rather than a desktop shot squeezed.
 *
 * All at `deviceScaleFactor: 2`, so the file is twice the size it is displayed at and stays sharp
 * on the screens this product is for.
 *
 *   pnpm --filter @summo/web build
 *   cargo build --bin summo-engine --features bundled,models,dub
 *   node apps/web/e2e/site-shots.mjs ../summo-site/public/anh
 *
 * The names are the site's, in Vietnamese, because that is what `lib/content/*.ts` asks for. They
 * are the contract between the two repositories and this file is the only place they are written
 * down on this side.
 */
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(process.argv[2] ?? join(HERE, "../../../../summo-site/public/anh"));

const WIDE = { width: 1440, height: 920 };
const FEATURE = { width: 1120, height: 760 };
const PHONE = { width: 390, height: 844 };

/**
 * Every picture the site names, and how to take it.
 *
 * `at` is the route. `shape` is which viewport. `scheme` is the colour scheme — the gallery shows
 * two screens in dark mode, deliberately, and those two are the only ones.
 */
const SHOTS = [
  // The hero and the first feature row.
  { name: "trang-chinh", at: "/", shape: WIDE },
  { name: "kho", at: "/library", shape: FEATURE },
  { name: "viec", at: "/tasks", shape: FEATURE },
  { name: "cai-dat", at: "/settings", shape: FEATURE },
  // The gallery.
  { name: "tim-kiem", at: "/", shape: WIDE, palette: true },
  { name: "mo-hinh", at: "/models", shape: WIDE },
  { name: "thong-ke", at: "/analytics", shape: WIDE },
  { name: "giong-noi", at: "/people", shape: WIDE },
  { name: "lich", at: "/agenda", shape: WIDE },
  { name: "kho-toi", at: "/library", shape: WIDE, scheme: "dark" },
  { name: "cai-dat-toi", at: "/settings", shape: WIDE, scheme: "dark" },
  // The phone.
  { name: "dien-thoai-chinh", at: "/", shape: PHONE },
  { name: "dien-thoai-kho", at: "/library", shape: PHONE },
];

/** The meeting page, which has no route of its own until something is opened. */
const MEETING = { name: "trang-hop", shape: FEATURE };

if (!existsSync(OUT)) {
  console.error(`no directory at ${OUT} — is the site checked out beside this repository?`);
  process.exit(1);
}

// `cwebp` rather than Playwright's JPEG: these are flat interface screenshots, where WebP is a
// third of the size of a PNG at visually identical quality, and the site ships them as `.webp`.
try {
  execFileSync("cwebp", ["-version"], { stdio: "ignore" });
} catch {
  console.error("cwebp is not installed — `apt-get install webp` or `brew install webp`");
  process.exit(1);
}

/**
 * A vault with a speech model in it.
 *
 * Without one the app is right to say so — a strip across the top of every screen reading
 * "Mô hình nhận dạng — cần để ghi được · Cài ngay" — and it is not dismissable, because an install
 * that cannot transcribe is not a thing to hide from somebody using it. In a screenshot it reads as
 * a product with a permanent error in it, and it was in all thirteen pictures on the landing page.
 *
 * So the model is installed rather than the warning suppressed: the shots are of a working install
 * because the install works. From the local mirror the other suites use, so this needs the network
 * once per machine and never again.
 */
const local = await mirror(["gipformer-65m", "silero-vad-v5"], { name: "site-shots" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("cannot photograph a working install without the model that makes it work");
  process.exit(1);
}

const engine = await boot({ name: "site-shots", registry: local.registry });

/** Install a model through the daemon, and wait for it. */
async function install(id) {
  const at = (path) => `${engine.url}${path}${path.includes("?") ? "&" : "?"}token=${engine.token}`;
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 300; i += 1) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((candidate) => candidate.model === id);
    if (job?.state === "done") return;
    if (job?.state === "failed") throw new Error(`${id}: ${job.error ?? "install failed"}`);
    await new Promise((resolve) => setTimeout(resolve, 400));
  }
  throw new Error(`${id}: still installing after two minutes`);
}

for (const id of ["silero-vad-v5", "gipformer-65m"]) {
  console.log(`installing ${id}…`);
  await install(id);
}

/**
 * A third meeting, for the board.
 *
 * The shared fixture in `daemon.mjs` is written for the suites that assert on it, and every one of
 * its four tasks is in the first lane — which is correct for those suites and makes the task board
 * a picture of one full column and three saying "nothing here". Written here rather than added
 * there: the assertions in `tasks.mjs` and `body.mjs` count what that fixture holds, and a
 * screenshot is not a reason to move somebody else's test.
 */
{
  const day = new Date();
  day.setDate(day.getDate() - 3);
  const on = day.toISOString().slice(0, 10);
  const meetings = join(engine.home, "vault/meetings");
  mkdirSync(meetings, { recursive: true });
  writeFileSync(
    join(meetings, `${on}-ra-mat.md`),
    `---
id: 01SITE1
date: ${on}T14:00:00+07:00
duration: 3120
participants: ["[[Bạn]]", "[[Ngọc]]", "[[Bình]]"]
tags: [ra-mắt, sản-phẩm]
color: violet
---

# Chuẩn bị ra mắt

## Tóm tắt
Chốt ngày ra mắt, chia việc trang chủ và bản dựng cho macOS.

## Việc cần làm
- [ ] @Ngọc Dựng trang chủ bản tiếng Anh <!-- id:01S1 status:doing -->
- [ ] @Bình Đo lại tốc độ trên máy M2 <!-- id:01S2 status:doing -->
- [ ] @Bạn Chờ Apple duyệt chứng chỉ <!-- id:01S3 status:blocked -->
- [x] @Ngọc Chốt tên bản phát hành <!-- id:01S4 status:done -->
- [x] @Bình Gửi bản thử cho ba khách quen <!-- id:01S5 status:done -->

## Transcript
**[00:02:10] Bạn** — Mình chốt ngày ra mắt trước đã <!-- seq:0 end:130.0 -->
**[00:04:45] Ngọc** — Trang chủ em làm xong bản tiếng Việt rồi <!-- seq:1 end:285.0 -->
**[00:06:20] Bình** — Bản macOS em đo lại tốc độ hôm nay <!-- seq:2 end:380.0 -->
`,
  );
}
const browser = await chromium.launch();

/** PNG in, WebP out, and the PNG removed. */
function toWebp(png, webp) {
  execFileSync("cwebp", ["-q", "88", "-quiet", png, "-o", webp]);
  rmSync(png);
}

async function open(shape, scheme = "light") {
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: shape,
    deviceScaleFactor: 2,
    colorScheme: scheme,
    // The pointer matters: the app draws a bottom bar below 768 px and a sidebar above it, and a
    // desktop context at phone width would get the wrong one.
    isMobile: shape === PHONE,
    hasTouch: shape === PHONE,
  });
  const page = await context.newPage();
  page.on("console", (m) => {
    if (m.type() === "error") console.error(`  console: ${m.text()}`);
  });
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
    waitUntil: "domcontentloaded",
  });
  await page.locator("h1, h2").first().waitFor({ timeout: 30000 });
  return { context, page };
}

/**
 * Dismiss the prompts a vault with nothing installed in it carries.
 *
 * The seeded vault has no speech model, so the app does the right thing and says so — a strip
 * across the top of every screen reading "Mô hình nhận dạng — cần để ghi được · Cài ngay". Correct
 * in the app, and in a marketing screenshot it reads as a product with a permanent error in it,
 * on all thirteen pictures.
 *
 * Dismissed rather than hidden with CSS: pressing the ✕ is a thing a person does, and what is left
 * is a screen that person is actually looking at. Nothing else about the shot is arranged.
 */
async function settle(page) {
  // After the screen has had time to ask for them: the strip is drawn from `/report`, so it is not
  // there when the first heading is, and dismissing at load dismissed nothing at all.
  for (let i = 0; i < 5; i += 1) {
    const close = page.getByRole("button", { name: /^Bỏ qua:/ }).first();
    if ((await close.count()) === 0) break;
    await close.click();
    await page.waitForTimeout(350);
  }
  // And the four-step tour, which a first-time user gets once and a screenshot should not.
  const skip = page.getByRole("button", { name: "Bỏ qua", exact: true });
  if ((await skip.count()) > 0) {
    await skip.first().click();
    await page.waitForTimeout(300);
  }
}

/** Where the screenshot lands, and what it is called. */
function paths(name) {
  return { png: join(OUT, `${name}.png`), webp: join(OUT, `${name}.webp`) };
}

for (const shot of SHOTS) {
  const { context, page } = await open(shot.shape, shot.scheme);
  await page.goto(`${engine.url}#${shot.at}`, { waitUntil: "domcontentloaded" });
  // Long enough for the lazily-fetched chunks and the entry animations, which is what a shot taken
  // too early catches half of.
  await page.waitForTimeout(2200);
  await settle(page);

  // The one picture that is of a thing rather than a screen: the command palette, over the app.
  if (shot.palette) {
    await page.keyboard.press("Control+k");
    await page.waitForTimeout(400);
    await page.keyboard.type("ngân sách", { delay: 40 });
    await page.waitForTimeout(900);
  }

  // Nothing may be photographed with a fault strip across the top of it. That strip is honest and
  // it is not dismissable, so its presence here means the vault is wrong rather than the shot —
  // which is exactly how thirteen pictures of a broken-looking install reached the landing page.
  if ((await page.getByText("cần để ghi được").count()) > 0) {
    throw new Error(`${shot.name}: the vault has no speech model and the screen says so`);
  }

  const { png, webp } = paths(shot.name);
  await page.screenshot({ path: png });
  toWebp(png, webp);
  console.log(`${shot.name}.webp — ${shot.at} at ${shot.shape.width}×${shot.shape.height}`);
  await context.close();
}

// The meeting, reached the way a person reaches it.
{
  const { context, page } = await open(MEETING.shape);
  await page.goto(`${engine.url}#/library`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(1500);
  await settle(page);
  await page.getByText("Họp đầu tuần").first().click();
  await page.waitForTimeout(2200);
  const { png, webp } = paths(MEETING.name);
  await page.screenshot({ path: png });
  toWebp(png, webp);
  console.log(`${MEETING.name}.webp — an open meeting`);
  await context.close();
}

// The setup screen, which is the one the site had most wrong: it was redrawn entirely and the
// picture on the page was of the version before. It only shows on a vault that has never been
// used, so it gets a daemon of its own with nothing in it.
{
  const fresh = await boot({ name: "site-shots-fresh", seed: false, onboarded: false });
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: FEATURE,
    deviceScaleFactor: 2,
  });
  const page = await context.newPage();
  await page.goto(`${fresh.url}?port=${fresh.port}&token=${fresh.token}`, {
    waitUntil: "domcontentloaded",
  });
  await page.locator("h1").first().waitFor({ timeout: 30000 });
  await page.waitForTimeout(2000);
  const { png, webp } = paths("cai-dat-lan-dau");
  await page.screenshot({ path: png });
  toWebp(png, webp);
  console.log("cai-dat-lan-dau.webp — the first-run setup screen");
  await context.close();
  fresh.stop();
}

// The social card.
//
// It was a crop of the home screen — a picture with no words on it, which is what a link to this
// product looks like in every chat app, timeline and search result that renders one. A card should
// say the name and the claim, because that is the whole of what a reader gets before deciding
// whether to click.
//
// Composed here rather than drawn by hand in an image editor: it is made of the same screenshot
// that was just taken, so it cannot go stale on its own.
{
  // Inlined rather than linked. A page built with `setContent` has no origin, so a `file://`
  // image on it is refused and lands as a broken-image icon in the middle of the card — which is
  // exactly what the first run of this produced.
  const shot = `data:image/webp;base64,${readFileSync(join(OUT, "trang-chinh.webp")).toString("base64")}`;
  const context = await browser.newContext({
    viewport: { width: 1200, height: 630 },
    deviceScaleFactor: 1,
  });
  const page = await context.newPage();
  await page.setContent(`
    <html><body style="margin:0">
      <div style="width:1200px;height:630px;display:flex;flex-direction:column;
                  background:#f7f6f4;font-family:system-ui,-apple-system,'Segoe UI',sans-serif;
                  color:#17161a;overflow:hidden">
        <div style="padding:52px 60px 0">
          <div style="display:flex;align-items:center;gap:10px;font-size:21px;font-weight:600">
            <span style="display:inline-flex;gap:3px;align-items:flex-end;height:22px">
              <span style="width:4px;height:11px;background:#0f7350;border-radius:2px"></span>
              <span style="width:4px;height:22px;background:#0f7350;border-radius:2px"></span>
              <span style="width:4px;height:16px;background:#0f7350;border-radius:2px"></span>
            </span>
            Summo
          </div>
          <div style="margin-top:26px;font-size:52px;line-height:1.06;font-weight:600;
                      letter-spacing:-0.02em;max-width:660px">
            Ghi chú cuộc họp<br><span style="color:#0f7350">chạy trên máy bạn.</span>
          </div>
          <div style="margin-top:18px;font-size:20px;color:#56545e;max-width:600px">
            Nhận dạng giọng nói và tách người nói chạy cục bộ. Mã nguồn mở, AGPL-3.0.
          </div>
        </div>
        <img src="${shot}" style="position:absolute;left:640px;top:180px;width:820px;
             border-radius:16px;border:1px solid #e6e3de;box-shadow:0 24px 60px -20px #17161a40">
      </div>
    </body></html>`);
  await page.waitForTimeout(600);
  const png = join(OUT, "og.png");
  await page.screenshot({ path: png });
  console.log("og.png — 1200×630");
  await context.close();
}

await browser.close();
engine.stop();

console.log(`\nwritten to ${OUT}`);
