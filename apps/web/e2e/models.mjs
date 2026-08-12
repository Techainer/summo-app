/**
 * The model catalogue, on screen.
 *
 * The registry has always been able to answer this and nothing ever asked it — the only way to
 * install a model that was not the recommended one was `summo pull` on a command line. This checks
 * the screen that fixes that, and specifically the two things a card has to say *before* somebody
 * spends several hundred megabytes: how big it is, and whether the licence means the download goes
 * somewhere other than us.
 *
 * Points at the local registry directory, so the suite does not depend on a deployed one.
 */
import { chromium } from "playwright";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REGISTRY = join(HERE, "../../../../summo-registry");

const problems = [];
const fail = (message) => problems.push(message);

// The two models this suite installs are served from this machine. Reaching github.com twice per
// run made a screen test fail whenever the network or the host felt like it.
const local = await mirror(["silero-vad-v5", "sense-voice-small"], { name: "models" });
const engine = await boot({ name: "models", registry: local.registry });
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
  colorScheme: "dark",
});
const page = await context.newPage();

try {
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/models`, {
    waitUntil: "networkidle",
  });
  await page.locator('[data-testid="models"]').waitFor({ timeout: 10000 });
  await page.waitForTimeout(800);

  const body = await page.locator('[data-testid="models"]').innerText();

  // Grouped by what the model does. "Which speech model" and "which translator" are different
  // questions asked at different times.
  // Case-insensitively: the headings are uppercased in CSS, and `innerText` reports what is
  // rendered. Matching the source casing passed by accident against the intro paragraph, which
  // happens to contain the same words.
  const shouty = body.toLocaleUpperCase("vi");
  for (const heading of ["Nhận dạng giọng nói", "Dịch"]) {
    if (!shouty.includes(heading.toLocaleUpperCase("vi"))) fail(`no section for ${heading}`);
  }

  // Every model the registry knows, not only the speech ones the setup screen offers.
  for (const id of ["gipformer-65m", "small100", "silero-vad-v5", "campplus-sv"]) {
    if (!body.includes(id)) fail(`${id} is missing from the catalogue`);
  }

  // Size before you commit to it.
  if (!/\d+\s*MB|\d+(\.\d+)?\s*GB/.test(body)) {
    fail("no download size on any card");
  }

  // The licence, and the flag that says the bytes come from somewhere other than us. Finding that
  // out at the download is finding out after committing.
  if (!body.includes("MIT")) fail("no licence shown");
  if (!body.includes("upstream")) {
    fail("a model Summo does not host is not marked as coming from upstream");
  }

  const install = page.getByRole("button", { name: "Cài", exact: true });
  if ((await install.count()) === 0) fail("nothing can be installed from this screen");

  await page.screenshot({ path: "/tmp/shots/models.png", fullPage: true });

  // Install, then remove. These are 73 MB to 2.5 GB each and installing the wrong one is the most
  // likely mistake this screen invites, so the way back has to be on it.
  const vad = page.locator("article", { hasText: "silero-vad-v5" });
  await vad.getByRole("button", { name: "Cài", exact: true }).click();
  await page.waitForTimeout(400);
  // Two minutes, matching the sense-voice wait below. This is a download over a real HTTP client,
  // and how long it takes is a fact about the machine — this suite failed at 60 s only when it ran
  // last in a queue of eleven browsers. A timeout that measures load rather than behaviour is a
  // test that fails for the wrong reason.
  await vad.getByText("Đã cài").waitFor({ timeout: 120000 });

  // Two clicks, not a dialog: re-downloading a gigabyte is a real cost, and a modal is one more
  // thing to dismiss while tidying up several models.
  await vad.getByRole("button", { name: "Xoá", exact: true }).click();
  await vad.getByRole("button", { name: "Xoá?", exact: true }).click();
  await page.waitForTimeout(800);
  if ((await vad.getByText("Đã cài").count()) !== 0) {
    fail("a removed model is still shown as installed");
  }

  // Installing a model and then having no way to say "use this one" is what made the catalogue
  // decorative: the interface used to send a hardcoded `gipformer-65m`, so installing a Japanese
  // model changed nothing about what recording reached for.
  const sense = page.locator("article", { hasText: "sense-voice-small" });
  await sense.getByRole("button", { name: "Cài", exact: true }).click();
  await sense.getByText("Đã cài").waitFor({ timeout: 120000 });
  await sense.getByRole("button", { name: "Dùng", exact: true }).click();
  await sense.getByText("Đang dùng").waitFor({ timeout: 10000 });

  // And it reached the settings file, not only the screen.
  const settings = await page.evaluate(
    async ({ port, token }) =>
      await (await fetch(`http://127.0.0.1:${port}/settings?token=${token}`)).json(),
    { port: engine.port, token: engine.token },
  );
  if (settings?.settings?.models?.live !== "sense-voice-small") {
    fail(
      `choosing a model did not reach the settings: ${JSON.stringify(settings?.settings?.models)}`,
    );
  }

  // The daemon refuses to remove a model the settings point at, because the alternative is a
  // recording that fails to start much later with nothing connecting the two.
  // Passed in rather than read from the URL: the app strips `port` and `token` during its
  // handshake, so by now they are gone from `location`.
  const refused = await page.evaluate(
    async ({ port, token }) => {
      await fetch(`http://127.0.0.1:${port}/settings/llm?token=${token}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          provider: "ollama",
          translator: { provider: "local", model: "small100" },
        }),
      });
      const response = await fetch(`http://127.0.0.1:${port}/models/small100?token=${token}`, {
        method: "DELETE",
      });
      return { ok: response.ok, body: await response.text() };
    },
    { port: engine.port, token: engine.token },
  );
  if (refused.ok || !refused.body.includes("translation")) {
    fail(`removing the model in use was not refused with a reason: ${JSON.stringify(refused)}`);
  }

  // An unreachable registry is a state, not a blank screen: this is an app expected to work on a
  // plane. Simulated by refusing the request the catalogue makes.
  await page.route("**/catalogue*", (route) => route.abort());
  await page.reload({ waitUntil: "networkidle" });
  await page.waitForTimeout(600);
  const offline = await page.locator("body").innerText();
  if (!offline.includes("kho mô hình")) {
    fail("with the catalogue unreachable the screen says nothing about why it is short");
  }
} finally {
  await browser.close();
  engine.stop();
  await local.stop();
}

if (problems.length) {
  for (const problem of problems) console.error(`FAIL ${problem}`);
  process.exit(1);
}
console.log("models ok");
