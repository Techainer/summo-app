/**
 * Reconfiguring a meeting while it is running, driven the way a person drives it.
 *
 * `full-flow.mjs` proves audio becomes text and changes the language once. This proves the rest of
 * it, which is the part that did not exist until recently and is the part most easily broken by a
 * refactor: the model, the spoken language and the live translation can each be changed *during* a
 * recording, from inside the meeting, without ending it and without losing a line.
 *
 * Every claim here is checked against the daemon's own `/status` rather than against the interface
 * that just asked for the change. The two disagree exactly when it matters — a swap the daemon
 * refused leaves the old pipeline running and the dropdown showing the new value — and a suite that
 * believed the dropdown would pass while the feature was broken.
 *
 * The translation assertion is the expensive one and the reason this is a separate file: it needs
 * SMALL100, which is 610 MB. It is worth it. Live translation is a headline feature that shipped
 * without any end-to-end test at all, on the strength of the pieces being individually correct.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const wav = join(HERE, "fixtures/vi-fleurs.wav");
const problems = [];

// Two speech models, because switching between them is the point: Gipformer hears Vietnamese and
// nothing else, Whisper hears ninety-nine languages badly. A suite with one model can check that a
// dropdown exists and never that changing it does anything.
const MODELS = ["gipformer-65m", "silero-vad-v5", "whisper-tiny", "small100"];
const local = await mirror(MODELS, { name: "in-meeting" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this suite is about changing between models; it means nothing without them");
  process.exit(1);
}

const engine = await boot(process.argv, { name: "in-meeting", registry: local.registry });
const { url: appUrl, port, token } = engine;
const at = (path) => `${appUrl}${path}${path.includes("?") ? "&" : "?"}token=${token}`;
const status = async () => (await fetch(at("/status"))).json();

async function install(id) {
  await fetch(at("/installs"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
  for (let i = 0; i < 600; i++) {
    const jobs = await (await fetch(at("/installs"))).json();
    const job = jobs.find((candidate) => candidate.model === id);
    if (job?.state === "done") return;
    if (job?.state === "failed") throw new Error(`${id}: ${job.error ?? "install failed"}`);
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`${id}: still installing after 300s`);
}

for (const id of MODELS) {
  console.log(`installing ${id}…`);
  await install(id);
}

// Nothing is configured here on purpose, and that is the point of this line.
//
// This suite used to POST `llm.translator = small100` before recording, and in doing so it tested
// a setup no user performs: somebody who presses Install on the models screen has the model on
// disk and an empty `llm.translator`. In that state the daemon resolved translation to the default
// `ollama` provider, accepted "translate into English", reported success and then failed every
// line against a server that was never running — with the suite green throughout, because the
// suite had written the setting the user never writes.
//
// So the plan is *asked* instead, and it has to name the model already. `using` is the daemon
// answering "what would actually do this", which is a different question from "what does the
// settings file say" — see `translator_here` in `server.rs`.
{
  const plan = await (await fetch(at("/settings/plan"))).json();
  if (plan.translation.using !== "small100") {
    console.error(
      "installing SMALL100 is not enough to translate with it: " + JSON.stringify(plan.translation),
    );
    process.exit(1);
  }
}

// Pinned, so the swap below is a genuine change. Without it the ranking may already have chosen
// Whisper and the assertion would pass having swapped a model for itself.
await fetch(at("/settings/models"), {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ role: "live", model: "gipformer-65m" }),
});

const browser = await chromium.launch({
  args: [
    "--use-fake-ui-for-media-stream",
    "--use-fake-device-for-media-stream",
    `--use-file-for-fake-audio-capture=${wav}`,
    "--autoplay-policy=no-user-gesture-required",
  ],
});
// Vietnamese, because every label this suite selects by is Vietnamese. Without it the app honours
// the machine's locale — correctly — and every selector here misses.
const context = await browser.newContext({
  locale: "vi-VN",
  permissions: ["microphone"],
  viewport: { width: 1180, height: 820 },
  colorScheme: "dark",
});
const page = await context.newPage();
page.on("console", (message) => {
  if (message.type() === "error") problems.push(`console: ${message.text()}`);
});

await page.goto(`${appUrl}?port=${port}&token=${token}`, { waitUntil: "networkidle" });

console.log("clicking record…");
await page
  .getByTestId("home")
  .getByRole("button", { name: /Bắt đầu ghi/ })
  .click();

await page
  .locator('[data-testid="transcript-line"]')
  .first()
  .waitFor({ timeout: 120000 })
  .catch((error) => {
    console.log("--- daemon log ---\n" + engine.log().slice(-4000));
    throw error;
  });

// ---- the controls are in the meeting, and there is one of them ------------
{
  // Open already, without anybody pressing anything.
  //
  // They used to be behind a small "Đổi" link, and the report that made this an assertion was a
  // person on a live call saying the place to change the model, the language and the translation
  // had been removed. It had not; it was one unlabelled click away on the screen whose entire
  // purpose is running the meeting.
  const panel = page.getByTestId("listening-panel");
  await panel.waitFor({ timeout: 10000 });
  if ((await page.getByTestId("live-bar").getByTestId("listening-panel").count()) !== 1) {
    problems.push("the controls are not inside the recording bar — they are back in the chrome");
  }
  for (const control of ["Mô hình", "Ngôn ngữ nói", "Dịch trực tiếp"]) {
    if ((await panel.getByLabel(control, { exact: true }).count()) !== 1) {
      problems.push(`the live controls do not offer ${control}`);
    }
  }
  // And they can still be put away, by the same control that used to be the only way in.
  const toggle = page.getByTestId("live-bar").getByRole("button", { name: "Xong", exact: true });
  if ((await toggle.count()) !== 1) {
    problems.push("the controls cannot be collapsed again");
  }

  // The panel names the translator. With one installed there is no dropdown to say so, and saying
  // nothing is what let it offer a translation that resolved to an endpoint nobody was running.
  //
  // Polled, because the panel asks the daemon two questions when it opens and the answer here needs
  // both — the plan for *which* model, the catalogue for its name. Case-insensitively: what appears
  // is the catalogue's name, `SMALL100 · translation, 100 languages`, not the id.
  let note = "";
  for (let i = 0; i < 20 && !note.includes("small100"); i++) {
    note = (await panel.innerText()).toLowerCase();
    if (!note.includes("small100")) await page.waitForTimeout(250);
  }
  if (!note.includes("small100")) {
    problems.push(`the panel does not say what will translate: ${JSON.stringify(note)}`);
  }
  if (await page.getByLabel("Dịch trực tiếp").isDisabled()) {
    problems.push("translation is offered as unavailable with SMALL100 installed");
  }
  await page.screenshot({ path: "/tmp/shots/in-meeting-panel.png" });
}

/** Every assertion here is "the daemon agrees, and the meeting survived". */
async function settled(what, check) {
  for (let i = 0; i < 40; i++) {
    const now = await status();
    if (check(now)) return now;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  const now = await status();
  problems.push(`${what}: the daemon never agreed — ${JSON.stringify(now)}`);
  return now;
}

// ---- the model, mid-meeting ----------------------------------------------
{
  const before = await status();
  await page.getByLabel("Mô hình", { exact: true }).selectOption("whisper-tiny");
  const after = await settled("model swap", (s) => s.live_model === "whisper-tiny");
  console.log(`model: ${before.live_model} → ${after.live_model}, still ${after.state}`);
  if (after.state !== "recording") problems.push(`the meeting ended when the model changed`);
  if (after.segments < before.segments) {
    problems.push(
      `segments went backwards on the model swap: ${before.segments} → ${after.segments}`,
    );
  }

  // And the transcript is still one meeting, in order.
  //
  // A swap rebuilds the pipeline, and every sequence number and timestamp used to restart at zero
  // with it — so the next thing said was numbered the same as the first thing said, and the
  // transcript, which indexes by sequence number, overwrote its own beginning. On screen the clock
  // went back to 00:00 and new lines appeared at the top. Reported from a real call as "the time
  // keeps looping, and choosing the language again puts it back at the top".
  //
  // By sequence number, not by counting lines: an overwrite keeps the count.
  await page.waitForTimeout(8000);
  const seqs = await page
    .locator('[data-testid="transcript-line"]')
    .evaluateAll((nodes) => nodes.map((n) => Number(n.getAttribute("data-seq"))));
  const wrong = seqs.findIndex((seq, i) => i > 0 && seq <= seqs[i - 1]);
  if (seqs.length > 1 && wrong !== -1) {
    problems.push(`the transcript renumbered itself after the swap: ${seqs.join(",")}`);
  } else {
    console.log(`transcript after the swap: seq ${seqs.join(",")}`);
  }
}

// ---- the second speech model, mid-meeting --------------------------------
//
// It shipped with no live control at all: choosable on the models screen and nowhere else, so the
// one decision somebody makes *because of what they are hearing* — this call turned out to be half
// in English — could only be made before the call began. Recognition gets what translation got.
{
  const before = await status();
  await page.getByLabel("Mô hình phụ").selectOption("gipformer-65m");
  const on = await settled("second model on", (s) => s.refine_model === "gipformer-65m");
  console.log(
    `second model: ${before.refine_model ?? "(none)"} → ${on.refine_model}, still ${on.state}`,
  );
  if (on.state !== "recording") problems.push("the meeting ended when the second model was chosen");

  await page.getByLabel("Mô hình phụ").selectOption("");
  const off = await settled("second model off", (s) => s.refine_model === undefined);
  console.log(`second model off: ${JSON.stringify(off.refine_model)}, still ${off.state}`);
  if (off.state !== "recording")
    problems.push("the meeting ended when the second model was cleared");
}

// ---- a language the *model* supports, not one the ranking would pick ------
//
// The bug this pins down: the list used to come from "what would serve this language", which ranks
// Vietnamese to Gipformer — so with Whisper selected, Vietnamese was missing from a list of
// Whisper's own languages. Selecting it here only works if the list comes from the model.
{
  const picked = await page
    .getByLabel("Ngôn ngữ nói")
    .selectOption("ja")
    .then(() => true)
    .catch(() => false);
  if (!picked) problems.push("Japanese is not offered for Whisper, which supports it");
  const after = await settled("language swap", (s) => s.language === "ja");
  console.log(`language: → ${after.language}, still ${after.state}`);
  if (after.state !== "recording") problems.push("the meeting ended when the language changed");
  // Back to Vietnamese, so the translation below has real words to work on.
  await page.waitForTimeout(2000);
  await page.getByLabel("Ngôn ngữ nói").selectOption("vi");
  await settled("language back", (s) => s.language === "vi");
  await page.waitForTimeout(2000);
}

// ---- live translation, on and off, mid-meeting ----------------------------
{
  const linesBefore = await page.locator('[data-testid="transcript-line"]').count();
  await page.getByLabel("Dịch trực tiếp").selectOption("en");
  // A build without `mt-onnx` cannot run a translator in-process and says so. That is a property
  // of the binary, not a failure of the feature, so it is a loud skip rather than a red run — and
  // loud, because a silently skipped translation test is how translation shipped broken twice.
  const refused = await page
    .locator("text=/local-mt|in-process/")
    .first()
    .waitFor({ timeout: 4000 })
    .then(() => true)
    .catch(() => false);
  if (refused) {
    console.log("SKIPPED translation: this binary has no in-process translator (needs mt-onnx).");
    console.log("  The release build has it — see FEATURES in scripts/bundle.sh.");
    await browser.close();
    await engine.stop();
    console.log(
      problems.length
        ? `\nPROBLEMS:\n  ${problems.join("\n  ")}`
        : "\nin-meeting ok (translation skipped)",
    );
    process.exit(problems.length ? 1 : 0);
  }

  // How many lines were already finished when translation was turned on. Every one has to end up
  // with a subtitle: turning translation on used to mean "from the next sentence", and the banner
  // reported translation was on over a transcript with nothing translated in it.
  const earlier = await page
    .locator('[data-testid="transcript-line"][data-source="final"]')
    .count();

  const on = await settled("translate on", (s) => (s.translate_into ?? []).includes("en"));
  console.log(`translate: → ${on.translate_into}, still ${on.state}`);
  if (on.state !== "recording") problems.push("the meeting ended when translation was turned on");

  // The real proof: a second line, in the target language, under an original. Nothing else in this
  // suite can tell a configured translator from a working one.
  // By test id, not by `[data-testid="transcript-line"] p[lang]`: the translation is a *sibling*
  // of the line, not a child, and that selector silently matched nothing — reporting translation
  // as broken while it was rendering correctly on screen.
  const translated = page.getByTestId("transcript-translation");
  const arrived = await translated
    .first()
    .waitFor({ timeout: 90000 })
    .then(() => true)
    .catch(() => false);
  if (!arrived) {
    console.log("--- daemon log ---\n" + engine.log().slice(-3000));
    problems.push("translation was accepted but no translated line ever reached the transcript");
  } else {
    const [original, subtitle] = [
      await page.locator('[data-testid="transcript-line"]').first().innerText(),
      await translated.first().innerText(),
    ];
    console.log(`translated line: ${JSON.stringify(subtitle.slice(0, 70))}`);
    // Under the original, never instead of it: anybody checking a subtitle against the speaker
    // needs both, and a translation that replaced the text would destroy the record of what was
    // actually said.
    if (!original.includes(subtitle.trim().slice(0, 20))) {
      // Only meaningful if the original is still there at all.
      const stillThere = await page.locator('[data-testid="transcript-line"]').count();
      if (stillThere < linesBefore)
        problems.push("lines disappeared when translation was turned on");
    }
  }

  // The backlog, filled in. Polled because it drains only into room the live path is not using, so
  // on a busy meeting it arrives a batch at a time behind current speech — which is the whole point
  // of keeping it separate from the live queue.
  if (earlier > 0) {
    let filled = 0;
    for (let i = 0; i < 120 && filled < earlier; i++) {
      await page.waitForTimeout(500);
      filled = await page.getByTestId("transcript-translation").count();
    }
    console.log(`backfilled ${filled}/${earlier} lines said before translation was on`);
    if (filled < earlier) {
      console.log("--- daemon log ---\n" + engine.log().slice(-2000));
      problems.push(
        `turning translation on left ${earlier - filled} earlier lines with no subtitle`,
      );
    }
  }

  await page.screenshot({ path: "/tmp/shots/in-meeting-translated.png" });

  // ---- a second target, added on top of the first --------------------------
  //
  // A meeting can have more than one reader, and the second subtitle is another pass through a
  // model that is already resident rather than another model. The control that used to be a single
  // dropdown made that a choice about whose language mattered.
  {
    const before = await page.getByTestId("transcript-translation").count();
    // Through the `+`, not the main dropdown. The main one *is* the answer — choosing a language
    // there replaces what is being translated into, which is what somebody with one subtitle
    // expects and what the previous design got wrong by leaving the box permanently on "Tắt".
    await page.getByLabel("Thêm một ngôn ngữ nữa").selectOption("ja");
    const both = await settled("second target", (s) => {
      const into = s.translate_into ?? [];
      return into.includes("en") && into.includes("ja");
    });
    console.log(`translate: → ${both.translate_into}, still ${both.state}`);
    if (both.state !== "recording")
      problems.push("the meeting ended when a second target was added");

    // The first target keeps working. Adding Japanese must not replace English, which is what a
    // control that holds one value would have done.
    let after = before;
    for (let i = 0; i < 120 && after <= before; i++) {
      await page.waitForTimeout(500);
      after = await page.getByTestId("transcript-translation").count();
    }
    if (after <= before) {
      console.log("--- daemon log ---\n" + engine.log().slice(-3000));
      problems.push(`a second target produced no extra subtitle: ${before} → ${after}`);
    } else {
      console.log(`subtitles: ${before} → ${after} with two targets`);
    }

    // And the control says what two targets now means. A line already in the language it would be
    // translated into is no longer translated into it, so two targets is each line into the other
    // one — and a control still reading "translate into X and Y" describes behaviour the daemon
    // stopped having. A user who picks their own spoken language and sees nothing happen has been
    // told nothing about why.
    // Waited for, not counted once. Everything above this polls the *daemon* — `settled` reads
    // `/status` — and this reads the *screen*, which re-renders on its own schedule. Asserting on
    // the count the instant the daemon agreed made this fail on a loaded CI runner and pass
    // everywhere else, which is the worst kind of red: a real assertion about a real property,
    // failing for a reason that has nothing to do with either.
    const note = page.getByText(/thứ tiếng còn lại/);
    const said = await note
      .first()
      .waitFor({ timeout: 15000 })
      .then(() => true)
      .catch(() => false);
    if (!said) {
      problems.push("two targets are set and nothing says translation runs both ways");
    } else if (!/↔/.test(await note.first().innerText())) {
      problems.push(`the both-ways note does not name the pair: ${await note.first().innerText()}`);
    } else {
      console.log(`both ways: ${(await note.first().innerText()).trim()}`);
    }

    // Dropping one leaves the other. The chip is the control, and its label says which it drops.
    await page
      .getByRole("button", { name: /Ngừng dịch sang/ })
      .first()
      .click();
    const one = await settled("dropped one target", (s) => (s.translate_into ?? []).length === 1);
    console.log(`translate: → ${one.translate_into}, still ${one.state}`);
    if (one.state !== "recording") problems.push("the meeting ended when a target was dropped");
    await page.screenshot({ path: "/tmp/shots/in-meeting-two-targets.png" });
  }

  // ---- minimised: the line, its translation, and nothing else ---------------
  //
  // Checked here because this is the only place in the suite where a translated line is on screen,
  // and a panel that shows the original without its translation is the failure worth catching.
  //
  // Either presentation is acceptable and the suite accepts whichever the browser gave. Document
  // Picture-in-Picture opens a real window — Playwright sees it as a second page, which is how this
  // reads it — and where the API is absent the panel pins to the corner of this one. What is *not*
  // acceptable is a button that does neither, which is what this catches.
  {
    await page.getByRole("button", { name: "Thu nhỏ", exact: true }).click();
    await page.waitForTimeout(1500);

    const floating = context.pages().find((other) => other !== page);
    const panel = floating ? floating.getByTestId("live-pip") : page.getByTestId("live-pip");

    const shown = await panel
      .first()
      .waitFor({ timeout: 10000 })
      .then(() => true)
      .catch(() => false);

    if (!shown) {
      problems.push("minimising a meeting showed nothing, floating or pinned");
    } else {
      // The newest line, which is not the one asserted on above — so assert on the shape: something
      // was said, and a translation arrives under it.
      //
      // Polled, because the panel shows the line *being* said. Its translation is a round trip
      // behind, so the newest line is briefly untranslated by design — a caption that waited for
      // both would be a caption that always lagged. What has to be true is that one turns up.
      const original = (await panel.getByTestId("pip-original").innerText()).trim();
      if (original.length === 0) problems.push("the minimised panel shows no line");

      let translated = 0;
      for (let i = 0; i < 60 && translated === 0; i++) {
        translated = await panel.getByTestId("pip-translation").count();
        if (translated === 0) await page.waitForTimeout(500);
      }
      if (translated === 0) {
        problems.push("no translation ever appeared under the line in the minimised panel");
      }

      if (floating) {
        // A window with none of this page's CSS renders as black text on white, which is worse
        // than not opening: the point of minimising is to be unobtrusive over somebody else's
        // screen. The theme comes across as an attribute on `<html>`, not as a class.
        const styled = await page.evaluate(() => {
          const w = documentPictureInPicture.window;
          const el = w?.document.querySelector('[data-testid="live-pip"]');
          return {
            sheets: w?.document.querySelectorAll('link[rel="stylesheet"], style').length ?? 0,
            // Compared against this page rather than against a colour written down here. The
            // question is "did it open in the same theme", and a literal would be a copy of the
            // palette that goes stale the first time anybody edits it.
            background: el ? w.getComputedStyle(el).backgroundColor : null,
            page: getComputedStyle(document.body).backgroundColor,
          };
        });
        if (styled.sheets === 0) problems.push("the floating window got none of the page's styles");
        if (styled.background !== styled.page) {
          problems.push(
            `the floating window opened in a different theme: ${styled.background} over ${styled.page}`,
          );
        }
        console.log(`floating: ${styled.sheets} stylesheet(s), ${styled.background}`);
      } else {
        // Pinned, and saying which rather than pretending it floats.
        if ((await page.getByText(/không hỗ trợ cửa sổ nổi/).count()) === 0) {
          problems.push("pinned to the page and nothing says why it is not floating");
        }
        console.log("pinned to the page: this browser has no floating-window support");

        // And the readout does not end up underneath it.
        //
        // Both were `fixed` in the bottom-right: the pinned meeting at up to 416 pixels wide, and
        // the resource readout at 224. Two panels in one corner means the stacking order decides
        // which one you can see, and being right about the order — the meeting wins — does not
        // make the readout being invisible correct. It is a different corner now, and this is what
        // stops it drifting back.
        //
        // Measured rather than reasoned about: `fixed` positions resolve against the viewport and
        // no amount of reading two class strings tells you whether two boxes touch.
        await page.evaluate(() => window.localStorage.setItem("summo.perf", "on"));
        await page.reload({ waitUntil: "networkidle" });
        await page.waitForTimeout(1500);

        const boxes = await page.evaluate(() => {
          const rect = (selector) => {
            const el = document.querySelector(selector);
            if (!el) return null;
            const { x, y, width, height } = el.getBoundingClientRect();
            return { x, y, width, height };
          };
          return { hud: rect('[data-testid="perf-hud"]'), pip: rect('[data-testid="live-pip"]') };
        });

        if (!boxes.hud) {
          problems.push("the readout was switched on and did not draw");
        } else if (boxes.pip) {
          const overlap =
            boxes.hud.x < boxes.pip.x + boxes.pip.width &&
            boxes.pip.x < boxes.hud.x + boxes.hud.width &&
            boxes.hud.y < boxes.pip.y + boxes.pip.height &&
            boxes.pip.y < boxes.hud.y + boxes.hud.height;
          if (overlap) {
            problems.push(
              `the readout and the minimised meeting overlap: ` +
                `readout ${JSON.stringify(boxes.hud)} against pip ${JSON.stringify(boxes.pip)}`,
            );
          } else {
            console.log("readout and minimised meeting: different corners, no overlap");
          }
        }
      }

      // Minimising is a change of view, not of state.
      const still = await settled(
        "still recording while minimised",
        (s) => s.state === "recording",
      );
      if (still.state !== "recording") problems.push("minimising ended the recording");

      console.log(
        `minimised: ${JSON.stringify(original.slice(0, 40))} with ${translated} translation(s)`,
      );
      // The panel itself, which on the floating path is a different page — screenshotting the one
      // underneath would photograph the thing the user just minimised.
      await (floating ?? page).screenshot({ path: "/tmp/shots/in-meeting-minimised.png" });

      // And back, leaving one of it rather than two.
      await panel.getByRole("button", { name: "Quay lại buổi họp" }).click();
      await page.waitForTimeout(1000);
      const left = context.pages().filter((other) => other !== page).length;
      if (left !== 0 || (await page.getByTestId("live-pip").count()) !== 0) {
        problems.push("restoring left the minimised panel open");
      }
    }
  }

  // Off is a state, not the absence of one. It could not be reached at all before: translation was
  // read once at session start, so a call that turned out not to need it paid for a translator on
  // every line until it ended.
  await page.getByLabel("Dịch trực tiếp").selectOption("");
  const off = await settled("translate off", (s) => s.translate_into === undefined);
  console.log(`translate off: ${JSON.stringify(off.translate_into)}, still ${off.state}`);
  if (off.state !== "recording") problems.push("the meeting ended when translation was turned off");
}

await page
  .getByRole("button", { name: /Dừng ghi/ })
  .first()
  .click();
await page.waitForTimeout(2000);

await browser.close();
await engine.stop();

console.log(problems.length ? `\nPROBLEMS:\n  ${problems.join("\n  ")}` : "\nin-meeting ok");
if (problems.length) process.exit(1);
