/**
 * The voice book, and the half of it that had never rendered.
 *
 * `People` took a `meeting` prop and asked "which voices in *this* recording have no name". Nothing
 * ever passed it one — the component is used in exactly one place, the voice book screen, with no
 * meeting — so the questions half was permanently empty and the screen was a read-only list of
 * people Summo already knew. The naming interface, which is the entire point of a voice book, was
 * unreachable from the interface.
 *
 * So the question is now asked of the whole vault, which is also the right question: a voice you
 * cannot place is one you go looking for, not one you happen to be looking at.
 */
import { chromium } from "playwright";

import { boot } from "./daemon.mjs";

const problems = [];
const engine = await boot({ name: "voices" });
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
});
const page = await context.newPage();
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));

try {
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/people`, {
    waitUntil: "networkidle",
  });

  const asking = page.getByTestId("unnamed-voices");
  await asking
    .waitFor({ timeout: 10000 })
    .catch(() => problems.push("the voice book asks about nothing"));

  if ((await asking.count()) > 0) {
    const shown = await asking.innerText();
    // The label alone is not a question anybody can answer — which conversation it was in is what
    // makes it answerable, so the meeting is named above it and links to the page.
    if (!shown.includes("S2")) problems.push(`the unnamed voice is missing: ${shown}`);
    if (!shown.includes("Demo khách hàng")) {
      problems.push(`the voice is not attributed to a meeting: ${shown}`);
    }

    // Naming it. The name is typed rather than picked, because on a fresh vault there is nobody in
    // the book to pick from — which is exactly the state a first user is in.
    await asking.getByRole("textbox").first().fill("Bình");
    await asking.getByRole("button", { name: "Lưu" }).first().click();
    await page.waitForTimeout(2000);

    const people = await (await fetch(`${engine.url}/people?token=${engine.token}`)).json();
    if (!people.people.some((who) => who.name === "Bình")) {
      problems.push(`naming the voice made nobody: ${JSON.stringify(people.people)}`);
    }

    // An answered question leaves the list.
    const left = await (await fetch(`${engine.url}/voices/unknown?token=${engine.token}`)).json();
    if (left.length !== 0) {
      problems.push(`the named voice is still being asked about: ${JSON.stringify(left)}`);
    }
  }
} finally {
  await browser.close();
  await engine.stop();
}

if (problems.length > 0) {
  console.error(problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log("voices ok");
