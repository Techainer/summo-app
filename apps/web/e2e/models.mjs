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

/**
 * Whether the bytes for a model actually made it onto this machine.
 *
 * Everything on this screen that does not involve installing is checked either way — the sizes,
 * the licences, the upstream marking, the button being there at all. Only the install-and-remove
 * pass needs the blob, and that pass is skipped rather than failed when github.com has refused a
 * runner: a red build over a screen with nothing wrong with it teaches people to rerun CI without
 * reading it, which costs more than this check is worth.
 *
 * Loudly, and never for everything at once. If nothing could be mirrored the network is the story
 * and the suite says so with a non-zero exit, because at that point it has checked almost nothing.
 */
const missing = new Set(local.unreachable.map((m) => m.id));
for (const { id, why } of local.unreachable) {
  console.log(`SKIPPED install/remove for ${id} — its bytes could not be fetched: ${why}`);
}
if (missing.size >= 2) {
  console.error("no model could be mirrored; this suite checked nothing that matters");
  process.exit(1);
}
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
  //
  // `gipformer-1.5-68m` rather than the 65M it replaces: a model the registry marks `superseded_by`
  // is not drawn unless it is installed. Two cards for one model, told apart by a parenthesis
  // inside one of the names, is what that rule ended.
  for (const id of ["gipformer-1.5-68m", "small100", "silero-vad-v5", "campplus-sv"]) {
    if (!body.includes(id)) fail(`${id} is missing from the catalogue`);
  }

  // And the one it replaced is not on the screen, because nothing here has it installed. A reader
  // choosing a model for Vietnamese should not have to work out which of two Gipformers is current.
  if (body.includes("gipformer-65m")) {
    fail("a model the registry says has been replaced is still offered beside its replacement");
  }

  // Size before you commit to it.
  if (!/\d+\s*MB|\d+(\.\d+)?\s*GB/.test(body)) {
    fail("no download size on any card");
  }

  // The licence, and the flag that says the bytes come from somewhere other than us. Finding that
  // out at the download is finding out after committing.
  if (!body.includes("MIT")) fail("no licence shown");
  // Who published it, on the card. The word "upstream" used to appear here only because the details
  // were expanded inline and the page text came with them; the credit line is the real marker, and
  // it is the one a person reads before spending a gigabyte.
  if (!/Của .+/.test(body)) {
    fail("no card says who published the model it is offering");
  }

  const install = page.getByRole("button", { name: "Cài", exact: true });
  if ((await install.count()) === 0) fail("nothing can be installed from this screen");

  await page.screenshot({ path: "/tmp/shots/models.png", fullPage: true });

  // ---- narrowing a catalogue that is now long enough to need it -----------
  //
  // Ten models over seven tasks: "which of these translates" used to be a scroll past everything
  // else. Search and the task chips narrow the same list, and the state where they match nothing
  // has to read as a typo rather than as a broken registry — that distinction is the whole reason
  // the empty state is not shared with the offline one.
  {
    const cards = page.locator("article");
    const search = page.getByTestId("model-search");

    const everything = await cards.count();
    await search.fill("small100");
    await page.waitForTimeout(300);
    const found = await cards.allInnerTexts();
    // Not "exactly one". The search reads descriptions too, and a translator whose description
    // compares itself to SMALL100 is a legitimate hit — hiding it would make the box worse. What
    // has to hold is that every card shown says the word, and that the shelf actually narrowed.
    if (found.length === 0) fail("searching for a model by its id found nothing");
    if (found.length >= everything) fail(`searching narrowed nothing: ${found.length} card(s)`);
    // The model itself is among them. Others can legitimately match — the search reads descriptions,
    // and a translator that compares itself to SMALL100 is a hit worth showing — but a search for an
    // id that does not return that id is a search box that lies.
    if (!found.some((card) => card.toLowerCase().includes("small100"))) {
      fail("searching for an id did not return the model with that id");
    }

    await search.fill("khong-co-mo-hinh-nao-ten-nhu-vay");
    await page.waitForTimeout(300);
    const dead = await page.locator('[data-testid="models"]').innerText();
    if (!dead.includes("Không có mô hình nào khớp")) {
      fail("a search that matches nothing does not say so");
    }
    if (dead.includes("Không kết nối được kho mô hình")) {
      fail("a search that matches nothing is reported as an unreachable registry");
    }

    const screen = page.getByTestId("models");
    await screen.getByRole("button", { name: "Bỏ bộ lọc", exact: true }).click();
    await page.waitForTimeout(300);
    if ((await cards.count()) < 2) fail("clearing the filters did not bring the catalogue back");

    // The task chips. `Dịch` is also a section heading, so this asks for the button specifically.
    await screen.getByRole("button", { name: "Dịch", exact: true }).click();
    await page.waitForTimeout(300);
    // The cards, not the whole pane: the panel at the top names every model a recording would use,
    // including the voice detector, and it is supposed to stay put while the shelf below narrows.
    const translators = (await cards.allInnerTexts()).join("\n");
    if (!translators.includes("small100")) fail("filtering to translation hid the translators");
    if (translators.includes("silero-vad-v5")) {
      fail("filtering to translation still shows the voice detector");
    }

    // Scoped to the catalogue: the folder tree in the sidebar has a "Tất cả" of its own, and an
    // unscoped selector matched both.
    await screen.getByRole("button", { name: "Tất cả", exact: true }).click();
    await page.waitForTimeout(300);
    if (!(await cards.allInnerTexts()).join("\n").includes("silero-vad-v5")) {
      fail("going back to every task did not restore the list");
    }
  }

  // Install, then remove. These are 73 MB to 2.5 GB each and installing the wrong one is the most
  // likely mistake this screen invites, so the way back has to be on it.
  if (!missing.has("silero-vad-v5")) {
    const vad = page.locator("article", { hasText: "silero-vad-v5" });
    await vad.getByRole("button", { name: "Cài", exact: true }).click();
    await page.waitForTimeout(400);
    // Two minutes, matching the sense-voice wait below. This is a download over a real HTTP
    // client, and how long it takes is a fact about the machine — this suite failed at 60 s only
    // when it ran last in a queue of eleven browsers. A timeout that measures load rather than
    // behaviour is a test that fails for the wrong reason.
    await vad.getByText("Đã cài").waitFor({ timeout: 120000 });

    // Two clicks, not a dialog: re-downloading a gigabyte is a real cost, and a modal is one more
    // thing to dismiss while tidying up several models.
    await vad.getByRole("button", { name: "Xoá", exact: true }).click();
    await vad.getByRole("button", { name: "Xoá?", exact: true }).click();
    await page.waitForTimeout(800);
    if ((await vad.getByText("Đã cài").count()) !== 0) {
      fail("a removed model is still shown as installed");
    }
  }

  // Installing a model and then having no way to say "use this one" is what made the catalogue
  // decorative: the interface used to send a hardcoded `gipformer-65m`, so installing a Japanese
  // model changed nothing about what recording reached for.
  if (!missing.has("sense-voice-small")) {
    const sense = page.locator("article", { hasText: "sense-voice-small" });

    // What the card promises, before the button is pressed.
    //
    // SenseVoice publishes an int8 export and a full-precision one. The card said 240 MB — the int8
    // figure — and the daemon downloaded **both**, 1.18 GB, because the install route passed
    // `variant: None`, whose documented meaning is "fetch whatever the manifest declares".
    // `variant::choose` had existed the whole time with tests, called by `summo pull` and by
    // nothing else, so installing from the command line fetched one build and installing from the
    // app fetched all of them. A user in Vietnam watched it die two thirds through a file they
    // were never told about.
    //
    // Asserted against the install job's own `total`, which is the number of bytes the daemon set
    // out to fetch — not the card, which would only be checking the interface against itself.
    const promised = await sense.innerText();
    await sense.getByRole("button", { name: "Cài", exact: true }).click();

    // Waiting on the job, not on one number out of it.
    //
    // This polled `total` and stopped when it was non-zero, which gives the same answer — zero —
    // for a job still being queued, a job whose first request has not answered yet, and a job that
    // failed outright. CI reported "the daemon fetches 0 MB" for thirty seconds of *something*,
    // and the sentence named the symptom of every possible cause.
    //
    // So: keep the last job seen, and say what state it was in when time ran out.
    let job = null;
    for (let i = 0; i < 240 && !(job?.total > 0); i++) {
      await page.waitForTimeout(250);
      const jobs = await (await fetch(`${engine.url}/installs?token=${engine.token}`)).json();
      job = jobs.find((j) => j.model === "sense-voice-small") ?? job;
      // A failure is terminal; waiting out the rest of the minute adds nothing but delay.
      if (job?.state === "failed") break;
    }
    const total = job?.total ?? 0;
    if (total === 0) {
      fail(
        `the install never reported a size: ${job ? `state ${job.state}, error ${job.error ?? "none"}` : "no job for sense-voice-small at all"}`,
      );
    }
    const mb = Math.round(total / 1e6);
    const onCard = Number(/(\d[\d.,]*)\s*MB/.exec(promised)?.[1]?.replace(/,/g, "") ?? 0);
    console.log(`sense-voice: card said ${onCard} MB, daemon fetches ${mb} MB`);

    // Two guarantees, and neither is a preference about precision.
    //
    // *One* build. SenseVoice's two exports total 1177 MB and the app fetched both, because the
    // install route passed `variant: None`. Which build wins is `variant::rank`'s business and it
    // is a measured decision rather than a taste — `docs/benchmarks.md` has whisper-tiny int8 at
    // 81.3 % against fp32's 67.6 % at identical speed — so pinning a number here would freeze a
    // policy this file is the wrong place to hold.
    if (mb >= 1100) {
      fail(`the app is downloading every build: ${mb} MB of a model whose builds total 1177 MB`);
    }
    // And the card says the same thing. It quoted the manifest's single `size_bytes` while the
    // installer fetched a different set of files, so one card carried 240 MB, 234 MB in its own
    // description, and 1.18 GB on the wire.
    if (Math.abs(onCard - mb) > 5) {
      fail(`the card promises ${onCard} MB and the daemon fetches ${mb} MB`);
    }
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
  }

  // ---- does it actually work -------------------------------------------
  //
  // "Đã cài" means a sha256 matched, which is a claim about a download rather than about a model.
  // Everything between those bytes and something that loads — a `params` key naming a file that is
  // not there, a variant resolving to a build that was never fetched, an archive unpacked into a
  // shape the runtime cannot open — was invisible until a recording started, and then surfaced as
  // a message about a hashed path in a shard directory.
  //
  // Driven on the card rather than against the route, because the route working and the card
  // showing nothing is the failure this is here to catch.
  if (!missing.has("sense-voice-small")) {
    const sense = page.locator("article", { hasText: "sense-voice-small" });
    await sense.getByRole("button", { name: "Kiểm tra", exact: true }).click();
    // Generous: this loads a 900 MB ONNX session and runs an inference, on a machine that may be
    // running ten other browsers. What is being measured is the answer, not the clock.
    const verdict = sense.getByTestId("check-sense-voice-small");
    await verdict.waitFor({ timeout: 180000 });
    const said = await verdict.innerText();
    // The model is real, installed and this build has the runtime, so the only correct answer is a
    // pass. A failure here is a genuine product failure and must not be tolerated as "some result
    // appeared".
    if (!said.includes("Chạy được")) {
      fail(`checking an installed model that works reported: ${said}`);
    }
    // And it says what happened rather than only that something did. A check whose whole output is
    // a tick is one nobody can act on when it turns red.
    if (said.trim().length < 20) fail(`the check result says nothing: ${said}`);
  }

  // A model this build has no runtime for is refused *before* the download, not after it.
  //
  // The release ships the ONNX translation runtime and not llama.cpp, so both GGUF translators in
  // the registry — 0.8 GB and 2.4 GB — were offered by every build that could never load them. The
  // download worked, the digest matched, and the failure arrived at the first translation as a
  // sentence about a compile-time flag.
  //
  // Asked of the route, because the point is that no client can spend those bytes: the card is one
  // caller, a stale page is another, and the cost of being wrong is measured in gigabytes.
  {
    const attempt = await page.evaluate(
      async ({ port, token }) => {
        const response = await fetch(`http://127.0.0.1:${port}/installs?token=${token}`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ id: "milmmt-46-1b" }),
        });
        return { ok: response.ok, body: await response.text() };
      },
      { port: engine.port, token: engine.token },
    );
    // Unless this build happens to have llama.cpp in it, in which case installing is correct and
    // there is nothing here to check. `mt-gguf` is not in any shipped feature set.
    const catalogue = await (await fetch(`${engine.url}/catalogue?token=${engine.token}`)).json();
    const gguf = catalogue.models?.find((m) => m.id === "milmmt-46-1b");
    if (gguf && gguf.runnable === false) {
      if (attempt.ok) {
        fail("a model this build has no runtime for started downloading anyway");
      } else if (!attempt.body.includes("runtime")) {
        fail(`the refusal does not say why: ${attempt.body}`);
      }
      if (!gguf.why_not) fail("the catalogue marks a model unrunnable without saying why");
    } else if (!gguf) {
      fail("milmmt-46-1b is missing from the catalogue");
    }
  }

  // Removing a model a role points at releases the role, rather than refusing.
  //
  // It used to refuse — `in use as the translation model; choose another one first` — and that is
  // advice nobody can take when there is no other model of that kind on the machine. SMALL100
  // installed, the settings pointing at it, 611 MB on disk, a Remove button that said no, and no
  // way out of the app. Reported twice from real use, the second time as "model vẫn chưa xóa
  // được".
  //
  // Pressing remove is not an accident. The role is un-pointed and the answer says what the next
  // recording will use, which is the question somebody who just deleted their recogniser has.
  //
  // This block asks about a model that is *not installed*, so the honest answer is that there is
  // nothing to remove — the role is released on the way through and the store reports the truth.
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
  if (refused.ok || !refused.body.includes("small100")) {
    fail(`removing a model that is not installed did not say so: ${JSON.stringify(refused)}`);
  }

  // And the case the complaint was actually about: a model that *is* installed and *is* in use.
  //
  // With one speech model on the machine, "choose another one first" is advice with no answer — so
  // the role is released and the reply names what the next recording will use. Asserted at the API
  // rather than through the card, because the interesting part is the state the daemon is left in:
  // a removal that left the settings pointing at a deleted model would trade this trap for the
  // missing-file failure the old guard existed to prevent.
  if (!missing.has("sense-voice-small")) {
    const gone = await page.evaluate(
      async ({ port, token }) => {
        const head = { "content-type": "application/json" };
        await fetch(`http://127.0.0.1:${port}/settings/models?token=${token}`, {
          method: "POST",
          headers: head,
          body: JSON.stringify({ role: "live", model: "sense-voice-small" }),
        });
        const response = await fetch(
          `http://127.0.0.1:${port}/models/sense-voice-small?token=${token}`,
          { method: "DELETE" },
        );
        const body = await response.json().catch(() => ({}));
        const plan = await (
          await fetch(`http://127.0.0.1:${port}/settings/plan?token=${token}`)
        ).json();
        return { ok: response.ok, body, still: plan.speech?.model ?? null };
      },
      { port: engine.port, token: engine.token },
    );
    if (!gone.ok) {
      fail(`a model in use could not be removed: ${JSON.stringify(gone)}`);
    } else if (!("now_using" in gone.body)) {
      fail(
        `the removal did not say what the next recording will use: ${JSON.stringify(gone.body)}`,
      );
    } else if (gone.still === "sense-voice-small") {
      fail("the settings still point at a model that is no longer on disk");
    } else {
      console.log(`removed a model in use; now using ${gone.body.now_using ?? "nothing"}`);
    }
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
