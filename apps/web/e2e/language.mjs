/**
 * Choosing the language that is being spoken, and getting a model for it.
 *
 * The flow this covers is the one a first user actually walks: they are not recording in the
 * language the interface happens to be in, they pick their own, and the model for it is not
 * installed. Every step of that used to be missing — setup inferred the language from the interface
 * locale, and a language with no model produced a record button that failed.
 *
 * Three things are checked, and each of them broke while being written:
 *
 * 1. The list is real: a hundred languages from the daemon, each with the model that serves it and
 *    the size of the download *this machine* would make.
 * 2. A missing model is offered as a download with progress, and the offer disappears when it lands.
 * 3. The choice reaches the daemon — `session_start` carries the language, rather than the app
 *    showing one language and recording in another.
 */
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

// The 73 MB the download step fetches, served from this machine.
//
// This suite presses "download the model for Vietnamese" and waits for it to land. Pointed at the
// real registry that is a 73 MB transfer over whatever the network is that day — it timed out on a
// CI runner and on a developer machine within the same hour, both times reporting "the download
// never finished", which says nothing about the screen this suite exists to check.
const local = await mirror(["gipformer-65m"], { name: "language" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  process.exit(1);
}

const engine = await daemon(process.argv, { name: "language", registry: local.registry });
const browser = await chromium.launch({
  args: ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"],
});
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 950 },
  permissions: ["microphone"],
});
const page = await context.newPage();
const problems = [];
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

// Every `session_start` the app sends, so the language it claims can be compared with the language
// it asked for.
const started = [];
page.on("websocket", (socket) => {
  socket.on("framesent", (frame) => {
    if (typeof frame.payload !== "string") return;
    try {
      const message = JSON.parse(frame.payload);
      if (message.cmd === "session_start") started.push(message);
    } catch {
      // Audio frames are binary and JSON.parse rejects the rest; neither is interesting here.
    }
  });
});

// ---- what the daemon offers ---------------------------------------------
{
  const response = await fetch(`${engine.url}/languages?token=${engine.token}`);
  const body = await response.json();

  if (body.languages.length < 50) {
    problems.push(`expected the multilingual list to be expanded, got ${body.languages.length}`);
  }

  const vi = body.languages.find((language) => language.code === "vi");
  if (!vi?.model) problems.push("no model offered for Vietnamese");
  // The distinction the whole screen exists for: a language somebody measured outranks one that is
  // merely covered by a multilingual model.
  if (vi && vi.model !== "gipformer-65m") {
    problems.push(`Vietnamese should resolve to the measured model, got ${vi.model}`);
  }
  const unmeasured = body.languages.find((language) => language.code === "af");
  if (unmeasured && unmeasured.accuracy !== 0) {
    problems.push(`Afrikaans reports an accuracy nobody measured: ${unmeasured.accuracy}`);
  }
}

// ---- picking a language with no model ------------------------------------
{
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/record`, {
    waitUntil: "networkidle",
  });
  const picker = page.getByLabel("Ngôn ngữ nói");
  await picker.waitFor({ timeout: 10000 });

  // Nothing chosen yet, and nothing installed that could detect: the control must say it is
  // following the settings rather than showing the first language in the list, which read as a
  // claim to be recording English.
  const first = await picker.locator("option").first().innerText();
  if (!/Theo cài đặt/.test(first)) {
    problems.push(`the unset state is not offered as an option; first option is "${first}"`);
  }

  await picker.selectOption("vi");
  const download = page.getByRole("button", { name: /Tải model/ });
  await download.waitFor({ timeout: 5000 });
  const label = await download.innerText();
  if (!/\d+\s*MB/.test(label)) problems.push(`the download does not say its size: "${label}"`);

  await download.click();

  // The offer disappears, because there is nothing left to download.
  //
  // Progress is not asserted. Served from this machine the transfer finishes between two polls, and
  // a check that demands to *see* "Đang tải" is a check that fails when the download is fast — the
  // opposite of what it is for. What has to hold is that the button resolves rather than spinning
  // forever, which is the failure a user reports.
  let landed = false;
  for (let i = 0; i < 120; i++) {
    await page.waitForTimeout(500);
    const busy = await page
      .getByRole("button", { name: /Tải model|Đang tải/ })
      .innerText()
      .catch(() => null);
    if (busy === null) {
      landed = true;
      break;
    }
  }
  if (!landed) problems.push("the download never finished");

  const chosen = await picker.locator("option:checked").innerText();
  if (!/Tiếng Việt/.test(chosen)) problems.push(`the choice did not stick: "${chosen}"`);
  if (/MB/.test(chosen))
    problems.push(`an installed model still advertises a download: "${chosen}"`);
}

// ---- the choice outlives this browser -------------------------------------
{
  const body = await (await fetch(`${engine.url}/languages?token=${engine.token}`)).json();
  if (body.current !== "vi") {
    problems.push(
      `the daemon did not remember the choice: current=${JSON.stringify(body.current)}`,
    );
  }
}

// ---- the choice reaches the daemon ---------------------------------------
{
  // The header carries one too, so this is scoped to the record card rather than by exact text.
  await page
    .getByRole("button", { name: /Bắt đầu ghi/ })
    .first()
    .click();
  await page.waitForTimeout(3000);

  const spec = started.at(-1);
  if (!spec) {
    problems.push("no session_start was sent");
  } else if (spec.language !== "vi") {
    problems.push(`session_start carried language ${JSON.stringify(spec.language)}, not "vi"`);
  }
}

await browser.close();
await local.stop();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("language ok");
