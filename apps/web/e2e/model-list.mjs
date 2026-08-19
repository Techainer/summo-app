/**
 * The model list on the setup screen, in every state it can be in.
 *
 * This is the screen a new user meets, and the one that decides whether the app is usable at all —
 * nothing records until something on it is installed. It broke twice in one day, in two different
 * ways, and neither was caught by anything:
 *
 * 1. **The daemon took minutes.** The registry chain was strictly sequential, and a Vietnamese ISP
 *    does not refuse a blocked address, it drops the packets — so two dead sources cost two full
 *    timeouts before the mirror that works was dialled. `registry.rs` covers that side now.
 * 2. **The screen called "still loading" a network failure.** The list is empty for the first
 *    moment of every healthy install, and the empty state said the ISP was probably blocking the
 *    download. A user sent a screenshot of it. It was also on the screenshot in the release notes,
 *    where it went unnoticed because the sentence looks plausible.
 *
 * So each state is driven here on purpose, in a real browser. The pathological ones are produced by
 * intercepting the request rather than by breaking the machine: a test that needs a blocked ISP to
 * reproduce is a test nobody runs.
 */
import { chromium } from "playwright";

import { boot } from "./daemon.mjs";

const engine = await boot({ name: "model-list", onboarded: false, seed: false });
const browser = await chromium.launch();
const problems = [];

/** Wording, in the locale the browser asks for below. */
const LOADING = "Đang lấy danh sách mô hình";
const BLOCKED = "Không lấy được danh sách mô hình";
const NONE_FOR_LANGUAGE = "Chưa có mô hình nào cho ngôn ngữ này";
const RECOMMEND = "**/onboarding/recommend*";

async function screen() {
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 900 },
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  return { context, page };
}

const open = (page) =>
  page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
    waitUntil: "domcontentloaded",
  });

const body = (page) => page.locator("main").innerText();

// ---- 1. it lists models, and never accuses the network on the way ---------
//
// Polled through the whole load rather than checked at the end. The bug was a state visible for a
// fraction of a second on a working install, so a check that only looks once it has settled is the
// check that missed it.
{
  const { context, page } = await screen();
  await open(page);
  await page.getByText("Chào mừng").first().waitFor({ timeout: 20000 });

  let sawBlocked = null;
  let listed = false;
  for (let i = 0; i < 100; i += 1) {
    const text = await body(page);
    if (text.includes(BLOCKED)) sawBlocked ??= `after ${i * 200}ms`;
    if ((await page.locator('input[type="radio"][name="model"]').count()) > 0) {
      listed = true;
      break;
    }
    await page.waitForTimeout(200);
  }

  if (!listed) problems.push("the model list never appeared against a working registry");
  if (sawBlocked) {
    problems.push(`a healthy install was told its network was blocked (${sawBlocked})`);
  }

  const rows = await page.locator('input[type="radio"][name="model"]').count();
  console.log(`healthy: ${rows} model(s) offered, blocked message never shown`);
  await context.close();
}

// ---- 2. while it is still being fetched, it says so ----------------------
{
  const { context, page } = await screen();
  // Held for three seconds, which is longer than any of the assertions below and shorter than the
  // patience of whoever is reading them.
  await page.route(RECOMMEND, async (route) => {
    await new Promise((r) => setTimeout(r, 3000));
    await route.continue();
  });
  await open(page);
  await page.getByText("Chào mừng").first().waitFor({ timeout: 20000 });

  // Watched while the request is in flight rather than waited on: what matters is everything the
  // screen says during those three seconds, and an assertion that throws on the first missing
  // string reports one symptom and hides the rest.
  let saidLoading = false;
  let accusedWhileLoading = false;
  let claimedNoModels = false;
  for (let i = 0; i < 25; i += 1) {
    const text = await body(page);
    if (text.includes(LOADING)) saidLoading = true;
    if (text.includes(BLOCKED)) accusedWhileLoading = true;
    if (text.includes(NONE_FOR_LANGUAGE)) claimedNoModels = true;
    if ((await page.locator('input[type="radio"][name="model"]').count()) > 0) break;
    await page.waitForTimeout(150);
  }

  if (!saidLoading) problems.push("a slow fetch never said it was fetching");
  if (accusedWhileLoading) {
    problems.push("a request still in flight was reported as a blocked network");
  }
  if (claimedNoModels) {
    problems.push("a request still in flight was reported as a language with no models");
  }

  // And it resolves into a list rather than staying on the message.
  await page
    .locator('input[type="radio"][name="model"]')
    .first()
    .waitFor({ timeout: 20000 })
    .catch(() => problems.push("the list never arrived after a slow fetch"));
  console.log("slow: says it is fetching, then lists");
  await context.close();
}

// ---- 3. when it genuinely fails, it says that, with the reason ----------
{
  const { context, page } = await screen();
  await page.route(RECOMMEND, (route) => route.abort("connectionrefused"));
  await open(page);
  await page.getByText("Chào mừng").first().waitFor({ timeout: 20000 });
  await page
    .getByText(BLOCKED)
    .first()
    .waitFor({ timeout: 20000 })
    .catch(() => problems.push("a fetch that failed outright said nothing about it"));

  const text = await body(page);
  if (text.includes(LOADING)) problems.push("a failed fetch still claims to be loading");
  // The detail line: what actually went wrong, under the sentence about the ISP. Without it the
  // screen makes a confident guess about somebody's network and offers nothing to check.
  const detail = await page.locator("main .text-micro").allInnerTexts();
  if (!detail.some((line) => line.trim().length > 0)) {
    problems.push("the failure gave no detail at all");
  }
  console.log("failed: says so, with a reason");
  await context.close();
}

// ---- 4. a language nothing covers is not a network problem --------------
{
  const { context, page } = await screen();
  await page.route(RECOMMEND, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ lang: "vi", models: [] }),
    }),
  );
  await open(page);
  await page.getByText("Chào mừng").first().waitFor({ timeout: 20000 });
  await page
    .getByText(NONE_FOR_LANGUAGE)
    .first()
    .waitFor({ timeout: 20000 })
    .catch(() => problems.push("a language with no models was not named as such"));

  if ((await body(page)).includes(BLOCKED)) {
    problems.push("an empty answer from a reachable daemon was reported as a blocked network");
  }
  console.log("empty: offers to change language, does not blame the ISP");
  await context.close();
}

// ---- 5. changing the language re-asks, and does not accuse anyone -------
//
// The exact sequence in the user's screenshot: the list is there, the language changes, the list is
// empty again for as long as the second request takes.
{
  const { context, page } = await screen();
  await open(page);
  await page.getByText("Chào mừng").first().waitFor({ timeout: 20000 });
  await page
    .locator('input[type="radio"][name="model"]')
    .first()
    .waitFor({ timeout: 20000 })
    .catch(() => problems.push("no list to change the language on"));

  // The second request is the slow one, so the gap between "asked" and "answered" is a real window
  // rather than a millisecond nobody can observe.
  await page.route(RECOMMEND, async (route) => {
    await new Promise((r) => setTimeout(r, 2500));
    await route.continue();
  });
  await page.getByLabel("Ngôn ngữ nói").selectOption("ja");

  let accused = false;
  for (let i = 0; i < 15; i += 1) {
    if ((await body(page)).includes(BLOCKED)) accused = true;
    await page.waitForTimeout(200);
  }
  if (accused) problems.push("changing the language reported the network as blocked");

  console.log("re-ask: language change does not flash a network error");
  await context.close();
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("model list ok");
