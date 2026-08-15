/**
 * Naming a voice — from the book, and from the line where you recognised it.
 *
 * Two things this covers.
 *
 * **The voice book's questions half had never rendered.** `People` took a `meeting` prop and asked
 * "which voices in *this* recording have no name". Nothing ever passed it one — the component is
 * used in exactly one place, the voice book screen, with no meeting — so the naming interface, the
 * entire point of a voice book, was unreachable. It asks about the whole vault now, which is also
 * the right question: a voice you cannot place is one you go looking for.
 *
 * **And the book is the wrong place to answer it.** You know who `S2` is because you are reading
 * what they said. Naming from the transcript is one click at the point of recognition, instead of a
 * screen, a scroll, and a label held in your head on the way.
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

const unnamed = async () =>
  (await (await fetch(`${engine.url}/voices/unknown?token=${engine.token}`)).json()).flatMap((m) =>
    m.voices.map((v) => `${m.meeting}/${v.label}`),
  );

try {
  // ---- the book asks about the whole vault --------------------------------
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/people`, {
    waitUntil: "networkidle",
  });

  const asking = page.getByTestId("unnamed-voices");
  await asking
    .waitFor({ timeout: 10000 })
    .catch(() => problems.push("the voice book asks about nothing"));

  const before = await unnamed();
  if (before.length !== 3) problems.push(`expected three unnamed voices, got ${before}`);

  if ((await asking.count()) > 0) {
    const shown = await asking.innerText();
    // The label alone is not a question anybody can answer — which conversation it was in is what
    // makes it answerable, so the meeting is named above it and links to the page.
    if (!shown.includes("S2")) problems.push(`the unnamed voice is missing: ${shown}`);
    if (!shown.includes("Demo khách hàng")) {
      problems.push(`the voice is not attributed to a meeting: ${shown}`);
    }
    // Newest first, so the recording you can still remember is at the top.
    if (shown.indexOf("Họp đầu tuần") > shown.indexOf("Demo khách hàng")) {
      problems.push(`the meetings are oldest-first: ${shown}`);
    }

    // Naming one. Typed rather than picked, because on a fresh vault there is nobody in the book to
    // pick from — which is exactly the state a first user is in.
    await asking.getByRole("textbox").first().fill("Ngọc");
    await asking.getByRole("button", { name: "Lưu" }).first().click();
    await page.waitForTimeout(2500);

    const people = await (await fetch(`${engine.url}/people?token=${engine.token}`)).json();
    if (!people.people.some((who) => who.name === "Ngọc")) {
      problems.push(`naming the voice made nobody: ${JSON.stringify(people.people)}`);
    }

    // The answered question leaves, and only it.
    const after = await unnamed();
    if (after.length !== 2 || after.includes("01E2E0/S2")) {
      problems.push(`naming 01E2E0/S2 left ${JSON.stringify(after)}`);
    }
  }

  // ---- and from the transcript, where the question is answerable ----------
  await page.goto(`${engine.url}?port=${engine.port}&token=${engine.token}#/pages/01E2E0`, {
    waitUntil: "networkidle",
  });
  await page.locator("h1").waitFor({ timeout: 10000 });
  await page.getByRole("radio", { name: "Bản ghi" }).click();
  await page.waitForTimeout(600);

  // `S3` is still a label. `Ngọc` was named a moment ago and must not be offered again — a name is
  // not a question.
  const speaker = page.getByRole("button", { name: "Đặt tên cho S3" });
  await speaker
    .waitFor({ timeout: 10000 })
    .catch(() => problems.push("the transcript offers no way to name its unnamed speaker"));
  if ((await page.getByRole("button", { name: "Đặt tên cho Ngọc" }).count()) > 0) {
    problems.push("a speaker who already has a name is still being asked about");
  }

  if ((await speaker.count()) > 0) {
    await speaker.click();
    const panel = page.getByTestId("name-voice");
    await panel.waitFor({ timeout: 5000 }).catch(() => problems.push("no naming panel opened"));

    // Somebody already in the book is one click, because by now there is one.
    const known = panel.getByRole("button", { name: "Ngọc", exact: true });
    if ((await known.count()) === 0) {
      problems.push("the people already in the book are not offered");
    } else {
      await known.click();
      await page.waitForTimeout(2500);

      // The transcript is rewritten by naming, so the chip has to stop saying `S3` without a
      // reload — the answer and the thing it changes are read back together.
      const rail = await page.locator("main").innerText();
      if (/\bS3\b/.test(rail)) problems.push(`the transcript still says S3: ${rail.slice(0, 300)}`);

      const left = await unnamed();
      if (left.length !== 1 || left[0] !== "01E2E1/S2") {
        problems.push(`naming from the transcript left ${JSON.stringify(left)}`);
      }
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
