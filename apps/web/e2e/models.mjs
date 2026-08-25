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

    let total = 0;
    for (let i = 0; i < 120 && total === 0; i++) {
      await page.waitForTimeout(250);
      const jobs = await (await fetch(`${engine.url}/installs?token=${engine.token}`)).json();
      total = jobs.find((job) => job.model === "sense-voice-small")?.total ?? 0;
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
