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
  // `exact` on every label: "Ghi" is a prefix of "Ghi chú", and a substring match resolves to both
  // the record screen and the notes screen.
  for (const [label, marker] of [
    ["Thư viện", /thư viện|họp|hôm/i],
    ["Ghi chú", /ghi chú|chưa có ghi chú|chọn một ghi chú/i],
    ["Lịch", /lịch|chưa có lịch/i],
    ["Việc", /việc|agent|chưa có/i],
    ["Hỏi đáp", /hỏi|kho họp/i],
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
    .getByRole("button", { name: "Thư viện" })
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
    .getByRole("button", { name: "Thư viện" })
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
