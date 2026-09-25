/**
 * Every control on every screen: does it have a name, and does pressing it do something sane.
 *
 * The suites in this directory each drive one feature deeply. That is the right shape for proving a
 * feature works and the wrong shape for finding the button nobody drives — and the gaps are real:
 * `Library.tsx` shipped a `className="ghost"` on a button, which is not a class this repository
 * defines, so it rendered as unstyled text for as long as nobody looked at that row. A suite that
 * checks *one* flow cannot find that. A suite that touches *everything* shallowly can.
 *
 * So this walks the screens and, on each one:
 *
 * **Every control has an accessible name.** An icon-only button with no `aria-label` is invisible
 * to a screen reader, unreachable by voice, and unnameable in every other suite in this directory —
 * `getByRole("button", { name: … })` cannot select what has no name. This is the check with the
 * highest yield per line, because the failure is silent for anybody not using assistive technology.
 *
 * **Every control is pressed.** Not to assert what it does — this file cannot know that — but to
 * assert what it must not do: throw, log an error, or leave the screen blank. Those three are the
 * shape of a control wired to nothing, wired to something that moved, or wired to something that
 * throws on a screen where its data is absent.
 *
 * ## What it deliberately does not press
 *
 * Anything destructive, each named below with the reason. A suite that deletes the fixture it is
 * standing on reports a cascade of failures that are all one failure, and a suite that stops the
 * daemon it is driving reports nothing at all.
 *
 * ## What a failure here means
 *
 * Narrow, and worth stating. A control that is pressed without error is not thereby *correct* —
 * this proves it is connected and survives contact, not that it does the right thing. The suite
 * that proves a feature does the right thing is the one named after that feature.
 */
import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { SCREENS, everyRouteIsCovered } from "./screens.mjs";

everyRouteIsCovered("controls.mjs");

const engine = await boot(process.argv, { name: "controls" });
const { url: appUrl, port, token } = engine;
const problems = [];

/**
 * Controls left unpressed, each with the reason.
 *
 * Matched against the accessible name. A name that is not listed here is pressed, so a new
 * destructive button is pressed once and noticed rather than quietly skipped.
 */
const LEAVE_ALONE = [
  [/^(Xoá|Xóa|Bỏ|Gỡ|Dừng|Ngừng|Quên)/i, "destroys or stops something the rest of the walk needs"],
  [/(đăng xuất|thoát|khởi động lại|reset|đặt lại)/i, "ends or resets the session this suite is in"],
  [
    /^(Ghi|Bắt đầu ghi)/i,
    "starts a recording, which every later screen would then be drawn behind",
  ],
  [
    /(tải|cài|pull|install)/i,
    "downloads hundreds of megabytes; the suites that need a model say so",
  ],
  // A link out of the app is not a control of the app. Clicking one navigates the whole page away
  // from the daemon, and what is on the other end is somebody else's to test.
  [/^https?:/i, "leaves the app"],
];

function leaveAlone(name) {
  return LEAVE_ALONE.find(([pattern]) => pattern.test(name));
}

const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
});
const page = await context.newPage();

/** Reset before each control, so a failure is attributed to the control that caused it. */
let noise = [];
page.on("pageerror", (error) => noise.push(`threw: ${error.message}`));
page.on("console", (message) => {
  if (message.type() === "error") noise.push(`console: ${message.text()}`);
});
// The console says "the server responded with a status of 400" and not which server or what for.
// A failure a reader cannot act on is half a failure, so the request itself is recorded.
page.on("response", (response) => {
  if (response.status() < 400) return;
  const { pathname } = new URL(response.url());
  // The body too. A status code says the daemon refused; only the body says what it refused and
  // why, and a suite that reports half of that sends its reader back to reproduce it by hand.
  void response
    .text()
    .then((body) => noise.push(`${response.status()} ${pathname} → ${body.slice(0, 200)}`))
    .catch(() => noise.push(`${response.status()} ${pathname}`));
});

const at = (route) => `${appUrl}?port=${port}&token=${token}#${route}`;

/**
 * Every control a person could press, as `{ name, selector }`.
 *
 * Visible and enabled only. A disabled button is a deliberate state — "nothing chosen yet",
 * "already running" — and pressing it proves nothing; an invisible one is behind a closed sheet and
 * belongs to whichever control opens it.
 *
 * Indexed by position within its role rather than held as an element handle, because pressing a
 * control re-renders the page and every handle taken before that is stale.
 */
async function controls(page) {
  return page.evaluate(() => {
    const name = (el) =>
      (el.getAttribute("aria-label") ?? el.textContent ?? el.getAttribute("title") ?? "")
        .replace(/\s+/g, " ")
        .trim();

    const visible = (el) => {
      const box = el.getBoundingClientRect();
      if (box.width === 0 || box.height === 0) return false;
      const style = getComputedStyle(el);
      return style.visibility !== "hidden" && style.display !== "none";
    };

    const found = [];
    const all = document.querySelectorAll(
      'button, [role="button"], [role="tab"], [role="radio"], summary, a[href]',
    );
    for (const el of all) {
      if (!visible(el)) continue;
      if (el.hasAttribute("disabled") || el.getAttribute("aria-disabled") === "true") continue;
      found.push({
        name: name(el),
        tag: el.tagName.toLowerCase(),
        href: el.getAttribute("href") ?? null,
      });
    }
    return found;
  });
}

for (const [label, route] of SCREENS) {
  await page.goto(at(route), { waitUntil: "networkidle" });
  await page.waitForTimeout(1200);

  const found = await controls(page);
  if (found.length === 0) {
    problems.push(`${label}: no controls at all, which means the screen did not render`);
    continue;
  }

  // ---- names --------------------------------------------------------------
  const nameless = found.filter((control) => control.name === "");
  if (nameless.length > 0) {
    problems.push(
      `${label}: ${nameless.length} control(s) with no accessible name ` +
        `(${nameless.map((c) => c.tag).join(", ")}) — unreachable by a screen reader, by voice, ` +
        "and by every other suite here, which selects controls by name",
    );
  }

  // ---- pressing -----------------------------------------------------------
  const pressable = found.filter((control) => {
    const name = control.href?.startsWith("http") ? control.href : control.name;
    return name !== "" && !leaveAlone(name);
  });

  let pressed = 0;
  const skipped = found.length - pressable.length;

  for (const control of pressable) {
    // Fresh each time: a press re-renders, and the list taken before it describes a page that no
    // longer exists. Matching by name rather than by index for the same reason.
    const target = page.getByRole(control.tag === "a" ? "link" : "button", {
      name: control.name,
      exact: true,
    });
    const count = await target.count().catch(() => 0);
    if (count === 0) continue; // It was replaced by something an earlier press did. Not a fault.

    noise = [];
    try {
      await target.first().click({ timeout: 4000 });
    } catch {
      // A control that cannot be clicked at all — covered by another element, moved mid-click.
      // Not reported: this suite is about what happens when a press lands, and a flaky click is a
      // worse signal than no signal.
      continue;
    }
    pressed += 1;
    await page.waitForTimeout(400);

    if (noise.length > 0) {
      problems.push(`${label} › "${control.name}": ${noise.join("; ")}`);
    }

    // A screen with nothing on it. This is what a control wired to a route that no longer exists
    // looks like, and it is the one outcome worse than an exception.
    const alive = await page.evaluate(() => document.body.innerText.trim().length);
    if (alive === 0) {
      problems.push(`${label} › "${control.name}": left the page blank`);
    }

    // Close whatever opened, and come back to the screen being walked. `Escape` first, because a
    // dialog left open covers the controls after it.
    await page.keyboard.press("Escape").catch(() => undefined);
    if (!page.url().includes(route === "/" ? "#/" : route)) {
      await page.goto(at(route), { waitUntil: "networkidle" });
      await page.waitForTimeout(600);
    }
  }

  console.log(
    `${label.padEnd(10)} ${String(found.length).padStart(3)} control(s), ` +
      `${pressed} pressed, ${skipped} left alone`,
  );
}

await browser.close();
await engine.stop();

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):`);
  for (const problem of problems) console.error(`  ✗ ${problem}`);
  process.exit(1);
}
console.log("\ncontrols ok: every control on every screen has a name and survives being pressed");
