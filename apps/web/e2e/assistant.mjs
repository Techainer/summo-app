/**
 * The assistant, beside the screen rather than instead of it.
 *
 * It was a destination you navigated away to, which is the wrong shape for the two things people
 * do with it: asking about the meeting currently open, and telling an agent to act while they keep
 * reading. This checks the panel opens from anywhere, leaves the screen visible on a wide window,
 * and takes the whole width on a narrow one — where there is no room for two columns.
 *
 * It does not check an answer or a run: both need a language model, and this suite has to pass on a
 * machine with none. What it checks is that the request is *made* and the failure is *shown* —
 * a swallowed error here is an assistant that appears to be thinking forever.
 */
import { chromium } from "playwright";

import { boot } from "./daemon.mjs";

const problems = [];
const fail = (message) => problems.push(message);

const engine = await boot({ name: "assistant" });
const browser = await chromium.launch();

try {
  const wide = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 860 },
    colorScheme: "dark",
  });
  const page = await wide.newPage();
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/library`, {
    waitUntil: "networkidle",
  });
  await page.locator('[data-testid="library"]').waitFor({ timeout: 10000 });

  await page.getByRole("button", { name: "Trợ lý" }).click();
  const panel = page.locator('[data-testid="assistant"]');
  await panel.waitFor({ timeout: 5000 });

  // Beside, not over: the whole point is that you can read the thing you are asking about.
  if (!(await page.locator('[data-testid="library"]').isVisible())) {
    fail("opening the assistant hid the screen it is meant to sit beside");
  }

  // Asking and acting are chosen, not guessed from grammar — guessing "do it" wrong writes to
  // somebody's vault.
  for (const mode of ["Hỏi", "Làm"]) {
    if ((await panel.getByRole("button", { name: mode, exact: true }).count()) === 0) {
      fail(`no ${mode} mode`);
    }
  }

  // The request is made and the failure is shown. No model is configured here, so the daemon
  // answers with an error, and that error has to reach the panel.
  await panel.getByRole("textbox").fill("ngân sách");
  await panel.getByRole("textbox").press("Enter");
  await page.waitForTimeout(8000);
  const said = await panel.innerText();
  if (!said.includes("ngân sách")) fail("the question was not echoed back");
  if (!/lỗi|error|Ollama|không/i.test(said)) {
    fail(`a failing request produced no visible message: ${JSON.stringify(said.slice(0, 200))}`);
  }

  await page.screenshot({ path: "/tmp/shots/assistant.png" });
  await wide.close();

  // On a phone there is no room for two columns, so the panel takes the width. Same component,
  // same state, one implementation.
  const narrow = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 390, height: 844 },
    colorScheme: "dark",
  });
  const phone = await narrow.newPage();
  await phone.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/library`, {
    waitUntil: "networkidle",
  });
  await phone.getByRole("button", { name: "Trợ lý" }).click();
  await phone.locator('[data-testid="assistant"]').waitFor({ timeout: 5000 });
  const box = await phone.locator('[data-testid="assistant"]').boundingBox();
  if (!box || box.width < 320) {
    fail(`the panel is ${box?.width ?? 0}px wide on a phone, which is a column beside nothing`);
  }
  await phone.screenshot({ path: "/tmp/shots/assistant-narrow.png" });
  await narrow.close();
} finally {
  await browser.close();
  engine.stop();
}

if (problems.length) {
  for (const problem of problems) console.error(`FAIL ${problem}`);
  process.exit(1);
}
console.log("assistant ok");
