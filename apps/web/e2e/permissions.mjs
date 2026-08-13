/**
 * The microphone permission panel, driven in a real browser.
 *
 * This is the one failure that cannot be reproduced in a unit test, because what fails is not our
 * code: `getUserMedia` rejects inside the browser, and everything worth checking — that the app
 * notices, that it names the browser *and* the operating system to fix, that it offers a way back —
 * happens in response to that rejection.
 *
 * Two halves, produced two different ways, and the difference is worth stating.
 *
 * **Granted** is real. Chromium is launched with a fake capture device and told to auto-accept, so
 * `getUserMedia`, the track it returns and `enumerateDevices` are all genuine.
 *
 * **Refused** is injected. On a machine with no audio backend — a CI container — Chromium does not
 * refuse, it answers `NotSupportedError`, which is a different state deserving different advice and
 * would test nothing here. So the denied context overrides `getUserMedia` and `permissions.query`
 * to behave exactly as a browser whose user pressed Block. That is honest about what this checks:
 * our response to a refusal, not the browser's ability to produce one.
 */
import { chromium } from "playwright";

import { daemon } from "./daemon.mjs";

const engine = await daemon(process.argv, { name: "permissions" });
const { url: appUrl, port, token } = engine;

const browser = await chromium.launch({
  args: ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"],
});
const problems = [];

/** What a browser looks like when its user pressed Block. */
const refuse = () => {
  navigator.mediaDevices.getUserMedia = () =>
    Promise.reject(new DOMException("Permission denied", "NotAllowedError"));
  navigator.permissions.query = (descriptor) =>
    descriptor && descriptor.name === "microphone"
      ? Promise.resolve({ state: "denied", onchange: null })
      : Promise.resolve({ state: "granted", onchange: null });
};

async function open({ allow }) {
  const context = await browser.newContext({
    locale: "vi-VN",
    viewport: { width: 1280, height: 900 },
    permissions: allow ? ["microphone"] : [],
  });
  if (!allow) await context.addInitScript(refuse);
  const page = await context.newPage();
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  // Hash routing, and the handshake goes in the query: the app reads `port`/`token` on load.
  await page.goto(`${appUrl}?port=${port}&token=${token}#/settings`, { waitUntil: "networkidle" });
  await page.locator('[data-testid="settings"]').waitFor({ timeout: 10000 });
  return { context, page };
}

// ---- refused -------------------------------------------------------------
{
  const { context, page } = await open({ allow: false });

  await page.getByRole("heading", { name: "Quyền micro" }).waitFor({ timeout: 10000 });
  await page.getByText("bị từ chối").waitFor({ timeout: 10000 });

  const steps = page.locator("ol li");
  const count = await steps.count();
  if (count < 3) problems.push(`expected browser + OS + retry steps, got ${count}`);

  const text = await page.locator("ol").innerText();
  // The instructions have to name a place, not a principle. Which operating system appears depends
  // on the machine running this, so any of the three is acceptable — "check your settings" is not.
  if (!/System Settings|Settings →|pavucontrol|PulseAudio/.test(text)) {
    problems.push(`instructions name no concrete place: ${text.slice(0, 160)}`);
  }
  if (!/ổ khoá|micro bị gạch|Safari/.test(text)) {
    problems.push(`instructions skip the browser's own block: ${text.slice(0, 160)}`);
  }
  if (!/Kiểm tra lại/.test(text)) {
    problems.push("instructions never say how to confirm the fix worked");
  }

  // A user whose platform was detected wrongly must still be able to read the right steps.
  await page.getByRole("button", { name: "macOS", exact: true }).click();
  const swapped = await page.locator("ol").innerText();
  if (!/System Settings/.test(swapped)) {
    problems.push(`switching to macOS did not change the instructions: ${swapped.slice(0, 160)}`);
  }

  await context.close();
}

// ---- granted -------------------------------------------------------------
{
  const { context, page } = await open({ allow: true });

  await page.getByRole("heading", { name: "Quyền micro" }).waitFor({ timeout: 10000 });

  // A context that already holds the permission reports `granted` on load, so there is nothing to
  // click — which is itself the right behaviour. Only press the button when it is offered.
  const ask = page.getByRole("button", { name: "Cho phép dùng micro" });
  if (await ask.isVisible().catch(() => false)) await ask.click();
  await page.getByText("đã cấp").waitFor({ timeout: 15000 });

  // Granted means no repair steps and nothing asking again — the panel becomes a statement.
  if ((await page.locator("ol li").count()) > 0) {
    problems.push("repair steps are shown even though the permission was granted");
  }
  if (await page.getByRole("button", { name: "Cho phép dùng micro" }).isVisible()) {
    problems.push("still asking for a permission that has been granted");
  }

  // Whatever the fake device is called, the count has to be real.
  const found = await page.getByText(/thiết bị vào/).innerText();
  if (/\b0 thiết bị vào/.test(found)) problems.push(`no input devices reported: ${found}`);

  await context.close();
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("permissions ok");
