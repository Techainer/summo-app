/**
 * The model list on the setup screen, in every state it can be in.
 *
 * This is the screen a new user meets, and the one that decides whether the app is usable at all —
 * nothing records until something on it is installed. It broke twice in one day, in two different
 * ways, and neither was caught by anything:
 *
 * 1. **The daemon took minutes.** The registry chain was strictly sequential, and a Vietnamese ISP
 *    does not refuse a blocked address, it drops the packets — so two dead sources cost two full
 *    timeouts before the mirror that works was dialled. `registry.rs` covers that side.
 * 2. **The screen called "still loading" a network failure.** The list is empty for the first
 *    moment of every healthy install, and the empty state said the ISP was probably blocking the
 *    download. A user sent a screenshot of it. It was also in the release notes, where it went
 *    unnoticed because the sentence looks plausible.
 *
 * Each state is driven on purpose, in a real browser. The pathological ones are produced by
 * intercepting the request rather than by breaking the machine: a test that needs a blocked ISP to
 * reproduce is a test nobody runs.
 *
 * **It runs against a daemon with no speech recognition in it, too.** CI builds
 * `--features bundled` and nothing else, so `/onboarding` reports `recognition: false`, the screen
 * offers no models at all, and the first version of this suite tested nothing there — it failed on
 * a picker that build never renders. The reply is patched to say the build can transcribe, which is
 * a claim about the binary rather than about this screen, and every state below is then reachable
 * on any build. The one thing that genuinely needs a recogniser — a real list from a real registry
 * — checks for it and says when it is skipped.
 */
import { chromium } from "playwright";

import { REGISTRY, boot } from "./daemon.mjs";

const engine = await boot({ name: "model-list", onboarded: false, seed: false });
const browser = await chromium.launch();
const problems = [];

/** Wording, in the locale the browser asks for below. */
const LOADING = "Đang lấy danh sách mô hình";
const BLOCKED = "Không lấy được danh sách mô hình";
const NONE_FOR_LANGUAGE = "Chưa có mô hình nào cho ngôn ngữ này";
// Regexes, not globs. Every request the app makes carries `?token=…`, so `**/onboarding` matched
// nothing at all — the patch below was installed, never fired, and the suite reported four passing
// states on a build that was rendering none of them. A route that silently matches nothing is the
// same class of bug as the one this file exists to catch.
const RECOMMEND = /\/onboarding\/recommend/;
const STATUS = /\/onboarding(\?|$)/;

/** Two plausible rows, for the states that must not depend on what a registry currently holds. */
const CANNED = {
  lang: "vi",
  models: [
    {
      id: "gipformer-65m",
      name: "Gipformer 65M · Vietnamese",
      accuracy: 0.91,
      expected_rtf: 0.06,
      gated: false,
      installed: false,
      license: "MIT",
      live_capable: true,
      reason: "91% accurate, 16× faster than real time on this machine",
      redistributable: true,
      size_bytes: 73218466,
    },
    {
      id: "whisper-tiny",
      name: "Whisper tiny (99 languages)",
      accuracy: 0.68,
      expected_rtf: 0.12,
      gated: false,
      installed: false,
      license: "MIT",
      live_capable: true,
      reason: "covers 99 languages",
      redistributable: true,
      size_bytes: 44000000,
    },
  ],
};

const recognises = await fetch(`${engine.url}/onboarding`, {
  headers: { authorization: `Bearer ${engine.token}` },
})
  .then((r) => r.json())
  .then((status) => status.recognition === true)
  .catch(() => false);

console.log(
  recognises
    ? "this daemon can transcribe: the real catalogue is checked as well"
    : "this daemon has no recogniser: the real catalogue is skipped, the screen states are not",
);

async function screen() {
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 900 },
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  // One field, and only on a build that lacks it. Everything else in the reply stays the daemon's
  // own answer about this machine.
  if (!recognises) {
    await page.route(STATUS, async (route) => {
      const response = await route.fetch();
      const status = await response.json();
      await route.fulfill({ json: { ...status, recognition: true } });
    });
  }
  return { context, page };
}

const open = (page) =>
  page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}`, {
    waitUntil: "domcontentloaded",
  });

const body = (page) => page.locator("main").innerText();
const rows = (page) => page.locator('input[type="radio"][name="model"]');
const welcome = async (page) => {
  await page.getByText("Chào mừng").first().waitFor({ timeout: 20000 });
  // The screen a build without recognition shows instead of the model step. If it is here, the
  // patch above did not take and everything below would pass by testing nothing.
  if ((await body(page)).includes("Bản dựng này không nhận dạng được")) {
    problems.push("the daemon was not persuaded it can transcribe: no model step to test");
  }
};

/**
 * What the screen says *while* a request is in flight, rather than after it settles.
 *
 * The bug was a sentence visible for as long as a fetch takes, so a check that only looks once
 * everything has arrived is the check that missed it.
 */
async function watch(page, ticks = 40, gap = 150) {
  const seen = { loading: false, blocked: false, none: false, listed: false };
  for (let i = 0; i < ticks; i += 1) {
    const text = await body(page);
    if (text.includes(LOADING)) seen.loading = true;
    if (text.includes(BLOCKED)) seen.blocked = true;
    if (text.includes(NONE_FOR_LANGUAGE)) seen.none = true;
    if ((await rows(page).count()) > 0) {
      seen.listed = true;
      break;
    }
    await page.waitForTimeout(gap);
  }
  return seen;
}

// ---- 0. the daemon says which kind of empty list this is -----------------
//
// Below the screen, because this is where the confusion started: `candidates` ranks whatever is
// installed when it cannot reach the catalogue, so an unreachable registry and a language nothing
// covers both come back `200 OK` with `models: []`. A user in Hanoi was told, in Vietnamese, that
// no model covers Vietnamese.
//
// Driven through the endpoint's own `registry` parameter, which replaces the whole chain — so this
// is a real daemon reading a real dead address, with no network and no interception.
{
  const ask = async (registry) => {
    const query = new URLSearchParams({ lang: "vi", token: engine.token, registry });
    return await fetch(`${engine.url}/onboarding/recommend?${query}`).then((r) => r.json());
  };

  // Port 9 is `discard`: nothing listens, and the connection is refused rather than hanging.
  const dead = await ask("https://127.0.0.1:9/registry");
  if (!dead.registry_error) {
    problems.push("an unreachable registry was reported as a perfectly good empty catalogue");
  } else if (!String(dead.registry_error).includes("127.0.0.1:9")) {
    problems.push(`the failure does not name the address it tried: ${dead.registry_error}`);
  }

  const good = await ask(REGISTRY);
  if (good.registry_error) {
    problems.push(`a readable registry reported an error: ${good.registry_error}`);
  }
  if (!Array.isArray(good.models) || good.models.length === 0) {
    problems.push("the local registry ranked nothing for Vietnamese");
  }

  console.log(
    `daemon: dead registry → ${dead.models?.length ?? "?"} model(s) and a reason, ` +
      `local registry → ${good.models?.length ?? "?"} model(s) and none`,
  );
}

// ---- 1. the real registry, on a build that can use one -------------------
if (recognises) {
  const { context, page } = await screen();
  await open(page);
  await welcome(page);

  const seen = await watch(page, 100, 200);
  if (!seen.listed) problems.push("the model list never appeared against a working registry");
  if (seen.blocked) problems.push("a healthy install was told its network was blocked");

  console.log(`real registry: ${await rows(page).count()} model(s) offered`);
  await context.close();
}

// ---- 2. a list that takes a moment says it is coming, not that it failed --
{
  const { context, page } = await screen();
  await page.route(RECOMMEND, async (route) => {
    await new Promise((r) => setTimeout(r, 2500));
    await route.fulfill({ json: CANNED });
  });
  await open(page);
  await welcome(page);

  const seen = await watch(page);
  if (!seen.loading) problems.push("a slow fetch never said it was fetching");
  if (seen.blocked) problems.push("a request still in flight was reported as a blocked network");
  if (seen.none) {
    problems.push("a request still in flight was reported as a language with no models");
  }
  if (!seen.listed) problems.push("the list never arrived after a slow fetch");

  console.log("slow: says it is fetching, then lists");
  await context.close();
}

// ---- 3. when it genuinely fails, it says that, with the reason -----------
{
  const { context, page } = await screen();
  await page.route(RECOMMEND, (route) => route.abort("connectionrefused"));
  await open(page);
  await welcome(page);
  await page
    .getByText(BLOCKED)
    .first()
    .waitFor({ timeout: 20000 })
    .catch(() => problems.push("a fetch that failed outright said nothing about it"));

  if ((await body(page)).includes(LOADING)) {
    problems.push("a failed fetch still claims to be loading");
  }
  // The detail line: what actually went wrong, under the sentence about the ISP. Without it the
  // screen makes a confident guess about somebody's network and offers nothing to check.
  const detail = await page.locator("main .text-micro").allInnerTexts();
  if (!detail.some((line) => line.trim().length > 0)) {
    problems.push("the failure gave no detail at all");
  }

  // And there is a way out of it that is not "quit the app": the network comes back, the button is
  // pressed, the list arrives.
  await page.unroute(RECOMMEND);
  await page.route(RECOMMEND, (route) => route.fulfill({ json: CANNED }));
  const retried = await page
    .getByRole("button", { name: "Thử lại" })
    .click({ timeout: 10000 })
    .then(() => true)
    .catch(() => false);
  if (!retried) {
    problems.push("a failed catalogue offered no way to ask again");
  } else {
    const back = await rows(page)
      .first()
      .waitFor({ timeout: 15000 })
      .then(() => true)
      .catch(() => false);
    if (!back) problems.push("asking again did not fetch the list");
  }

  console.log("failed: says so, with a reason, and can be asked again");
  await context.close();
}

// ---- 4. a language nothing covers is not a network problem ---------------
{
  const { context, page } = await screen();
  await page.route(RECOMMEND, (route) =>
    route.fulfill({
      json: {
        lang: "vi",
        models: [],
        // The daemon has always computed these and nothing ever showed them.
        rejected: [{ id: "whisper-tiny", reason: "does not cover vi (covers en)" }],
      },
    }),
  );
  await open(page);
  await welcome(page);
  await page
    .getByText(NONE_FOR_LANGUAGE)
    .first()
    .waitFor({ timeout: 20000 })
    .catch(() => problems.push("a language with no models was not named as such"));

  const text = await body(page);
  if (text.includes(BLOCKED)) {
    problems.push("an empty answer from a reachable daemon was reported as a blocked network");
  }
  // The dead end this used to be: "no model for this language" and nothing else. The daemon knows
  // which models it looked at and why each was dropped, and now so does the person reading.
  if (!text.includes("does not cover vi")) {
    problems.push("the reasons the candidates were rejected were not shown");
  }

  console.log("empty: names the models it rejected and why");
  await context.close();
}

// ---- 5. changing the language re-asks, and accuses nobody in the gap -----
//
// The sequence in the user's screenshot: a list is on screen, the language changes, and the list is
// empty again for as long as the second request takes.
{
  const { context, page } = await screen();
  await page.route(RECOMMEND, (route) => route.fulfill({ json: CANNED }));
  await open(page);
  await welcome(page);
  await rows(page)
    .first()
    .waitFor({ timeout: 20000 })
    .catch(() => problems.push("no list to change the language on"));

  // The second answer is the slow one, so the gap between asking and answering is a window somebody
  // could actually read rather than a millisecond nobody can.
  await page.unroute(RECOMMEND);
  await page.route(RECOMMEND, async (route) => {
    await new Promise((r) => setTimeout(r, 2000));
    await route.fulfill({ json: { ...CANNED, lang: "ja" } });
  });
  // Caught rather than thrown. A missing picker is a result to report next to the others, not an
  // exception that kills the run and takes the collected problems with it.
  const picked = await page
    .getByLabel("Ngôn ngữ nói")
    .selectOption("ja", { timeout: 10000 })
    .then(() => true)
    .catch(() => false);
  if (!picked) problems.push("the spoken-language picker was not on the setup screen");

  let accused = false;
  for (let i = 0; picked && i < 14; i += 1) {
    if ((await body(page)).includes(BLOCKED)) accused = true;
    await page.waitForTimeout(150);
  }
  if (accused) problems.push("changing the language reported the network as blocked");

  console.log("re-ask: language change does not flash a network error");
  await context.close();
}

// ---- 6. an empty list with a reason is a network problem, and says so ----
//
// The shape the user's machine actually produced: the daemon answers, the list is empty, and the
// reason is in the payload. Before this the screen picked the wrong one of its two sentences and
// told them to choose a different language.
{
  const { context, page } = await screen();
  await page.route(RECOMMEND, (route) =>
    route.fulfill({
      json: {
        lang: "vi",
        models: [],
        registry_error:
          "no registry source answered:\n  https://raw.githubusercontent.com/...: timed out",
      },
    }),
  );
  await open(page);
  await welcome(page);
  await page
    .getByText(BLOCKED)
    .first()
    .waitFor({ timeout: 20000 })
    .catch(() => problems.push("an empty list with a stated reason was not reported as a failure"));

  const text = await body(page);
  if (text.includes(NONE_FOR_LANGUAGE)) {
    problems.push("a blocked registry was reported as a language nothing covers");
  }
  if (!text.includes("raw.githubusercontent.com")) {
    problems.push("the reason the daemon gave was not shown to the user");
  }

  console.log("empty with a reason: named as a failure, with the address that failed");
  await context.close();
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("model list ok");
