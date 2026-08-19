/**
 * The application shell, driven in a real browser.
 *
 * Unit tests cover the folder tree and the report formatting; nothing but a browser catches a route
 * that does not render, a sidebar that will not collapse, or a stylesheet that loads but produces an
 * invisible interface. This walks every screen at two widths and in both colour schemes.
 */
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "shell" });
const { url: appUrl, port, token } = engine;

const browser = await chromium.launch();
const problems = [];
const shots = "/tmp/shots";

async function open(scheme, viewport) {
  // The suites assert Vietnamese wording, so the browser has to ask for Vietnamese. Without
  // this the app honours the machine's locale — which is exactly what it should do, and which made
  // every assertion here fail the moment translation landed.
  const context = await browser.newContext({ locale: "vi-VN", viewport, colorScheme: scheme });
  const page = await context.newPage();
  page.on("console", (m) => {
    if (m.type() === "error") problems.push(`console(${scheme}): ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror(${scheme}): ${e.message}`));
  await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });
  // React has not necessarily rendered by `networkidle`; wait for the shell itself.
  await page.locator("header").waitFor({ timeout: 10000 });
  return { context, page };
}

// ---- wide, dark ----------------------------------------------------------
{
  const { context, page } = await open("dark", { width: 1280, height: 820 });

  // The shell itself.
  await page.getByRole("navigation", { name: "Màn hình" }).waitFor({ timeout: 10000 });
  await page.getByRole("navigation", { name: "Thư mục", exact: true }).waitFor();

  // The folder tree must show the vault's own folders, not just the menu.
  const folders = await page
    .getByRole("navigation", { name: "Thư mục", exact: true })
    .getByRole("button")
    .allInnerTexts();
  console.log(`folders: ${folders.filter(Boolean).join(" | ")}`);
  // `khach-hang` is what `daemon.mjs` seeds. The assertion used to name a folder from whatever
  // vault this was first run against, which is why it only passed on one machine.
  if (!folders.some((f) => f.includes("khach-hang"))) {
    problems.push(`folder tree missing the seeded folder: ${JSON.stringify(folders)}`);
  }

  // The bundled faces must actually load: the app has to render Vietnamese offline, and a missing
  // face falls back to whatever the OS has, which on a bare machine drops tone marks entirely.
  const fonts = await page.evaluate(async () => {
    // A face is only fetched once something asks for it, so request each one explicitly rather
    // than checking whatever the current screen happened to use.
    const probe = async (spec) => {
      const loaded = await document.fonts.load(`16px ${spec}`, "Tiếng Việt ề ỗ ự");
      return loaded.length > 0;
    };
    return {
      inter: await probe("Inter"),
      beVietnam: await probe('"Be Vietnam Pro"'),
      mono: await probe('"JetBrains Mono"'),
    };
  });
  console.log(`fonts: ${JSON.stringify(fonts)}`);
  for (const [name, ok] of Object.entries(fonts)) {
    if (!ok) problems.push(`font ${name} did not load`);
  }

  // Stacked diacritics must stay inside their line box rather than clipping into the line above.
  const overflow = await page.evaluate(() => {
    const probe = document.createElement("p");
    probe.textContent = "ề ỗ ự ườ ẫ ẳ";
    probe.style.cssText = "position:fixed;top:0;left:0;font-size:15px;line-height:1.65;margin:0";
    document.body.append(probe);
    const { height } = probe.getBoundingClientRect();
    const scroll = probe.scrollHeight;
    probe.remove();
    return { height, scroll };
  });
  if (overflow.scroll > overflow.height + 1) {
    problems.push(`Vietnamese diacritics overflow their line box: ${JSON.stringify(overflow)}`);
  }

  // Every screen must render something rather than a blank frame.
  //
  // `exact` on every label. "Ghi" was a prefix of "Ghi chú" while both were in the sidebar, and a
  // substring match resolved to two screens; the notes row is gone now — notes live on the same
  // shelf as recordings — and `exact` stays because the next pair of labels will do it again.
  for (const [label, marker] of [
    ["Đã lưu", /mọi thứ|họp|hôm/i],
    ["Lịch", /lịch|chưa có lịch/i],
    ["Việc", /việc|agent|chưa có/i],
    ["Giọng nói", /giọng|chưa có ai/i],
    ["Thống kê", /thống kê/i],
    ["Cài đặt", /cài đặt|mô hình|llm|ngôn ngữ/i],
    ["Ghi", /ghi|đang nghe|bấm ghi/i],
  ]) {
    await page
      .getByRole("navigation", { name: "Màn hình" })
      .getByRole("button", { name: label, exact: true })
      .click();
    await page.waitForTimeout(400);
    const body = await page.locator("main").innerText();
    if (!marker.test(body)) problems.push(`screen ${label} rendered nothing recognisable`);
    console.log(`screen ${label}: ${body.slice(0, 60).replace(/\n/g, " ")}…`);
  }

  // Navigating twice in quick succession must leave the screen *visible*.
  //
  // It did not. The screen wrapper sat at `opacity: 0` — the exit variant — with the whole page
  // laid out behind it, because `AnimatePresence mode="wait"` was stranded by a second route
  // change arriving inside the 180 ms it holds the incoming child for.
  //
  // The loop above walks every screen and did not catch it, because `innerText` reports text
  // nobody can see. So this checks paint, not presence: every ancestor of the heading has to be
  // fully opaque.
  for (const [first, second] of [
    ["Đã lưu", "Việc"],
    ["Lịch", "Thống kê"],
  ]) {
    const nav = page.getByRole("navigation", { name: "Màn hình" });
    await nav.getByRole("button", { name: first, exact: true }).click();
    await nav.getByRole("button", { name: second, exact: true }).click();
    await page.waitForTimeout(600);
    const faded = await page.evaluate(() => {
      const start = document.querySelector("main h1") ?? document.querySelector("main");
      for (let node = start; node && node !== document.documentElement; node = node.parentElement) {
        const opacity = getComputedStyle(node).opacity;
        if (opacity !== "1")
          return `${node.tagName}.${String(node.className).slice(0, 24)} → ${opacity}`;
      }
      return null;
    });
    if (faded) {
      problems.push(`${first} then ${second} left the screen invisible: ${faded}`);
    }
  }
  await page.goBack();
  await page.waitForTimeout(600);

  // Analytics reads the report endpoint, which is arithmetic over the seeded vault.
  await page
    .getByRole("navigation", { name: "Màn hình" })
    .getByRole("button", { name: "Thống kê" })
    .click();
  await page.waitForTimeout(600);
  await page.screenshot({ path: `${shots}/shell-analytics.png` });

  // The sidebar collapses on demand and comes back.
  await page
    .getByRole("navigation", { name: "Màn hình" })
    .getByRole("button", { name: "Đã lưu" })
    .click();
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${shots}/shell-wide-dark.png` });
  await page.getByRole("button", { name: "Ẩn thanh bên" }).click();
  // Retry rather than sleep: the panel animates out, so a fixed wait either flakes or is too slow.
  try {
    await page
      .getByRole("navigation", { name: "Thư mục", exact: true })
      .waitFor({ state: "hidden", timeout: 4000 });
  } catch {
    problems.push("sidebar stayed reachable after being collapsed");
  }
  await page.getByRole("button", { name: "Hiện thanh bên" }).click();
  await page.getByRole("navigation", { name: "Thư mục", exact: true }).waitFor({ timeout: 4000 });

  await context.close();
}

// ---- narrow, light -------------------------------------------------------
{
  const { context, page } = await open("light", { width: 420, height: 820 });

  // Below the breakpoint the sidebar must start shut, or it covers the whole app on first paint.
  if (await page.getByRole("navigation", { name: "Thư mục", exact: true }).isVisible()) {
    problems.push("sidebar was open over the content on a narrow screen");
  }
  await page.screenshot({ path: `${shots}/shell-narrow-light.png` });

  // …and open as a sheet on demand, then shut itself once a choice is made.
  await page.getByRole("button", { name: "Hiện thanh bên" }).click();
  await page.waitForTimeout(400);
  await page.getByRole("navigation", { name: "Thư mục", exact: true }).waitFor();
  await page.screenshot({ path: `${shots}/shell-narrow-sheet.png` });
  await page
    .getByRole("navigation", { name: "Màn hình" })
    .getByRole("button", { name: "Đã lưu" })
    .click();
  try {
    await page
      .getByRole("navigation", { name: "Thư mục", exact: true })
      .waitFor({ state: "hidden", timeout: 4000 });
  } catch {
    problems.push("the sheet stayed open after navigating");
  }

  await context.close();
}

await browser.close();
engine.stop();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}
console.log("\nshell ok");
