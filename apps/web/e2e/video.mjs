/**
 * Importing a video, and then watching it back with subtitles.
 *
 * Every part of this was present and none of it joined up. Importing an `.mp4` extracted the audio
 * and left the video behind; the meeting screen drew an `<audio>` element and had no idea a source
 * file existed; and the one lane an imported meeting *did* have was called `import`, which the
 * audio route did not know about — so every imported meeting, video or not, rendered a transport
 * whose only track answered `no such lane 'import'`. Recorded meetings played back. Imported ones
 * never had.
 *
 * So this drives the whole chain from the outside:
 *
 * - an `.mp4` is imported through the daemon's own route, the way the app imports one,
 * - the meeting that comes out says where it came from and that it is a video,
 * - the byte-serving route answers a `Range` request, which is what makes a scrubber work,
 * - the meeting screen draws a **`<video>`**, not an `<audio>`,
 * - the imported audio lane plays, which is the regression that was invisible for releases,
 * - and the subtitle picker offers what was said, the translation, and both at once — with the
 *   browser's own parser confirming the cues, because a `.vtt` a player silently refuses looks
 *   exactly like one that works.
 *
 * The translation is written directly rather than produced by a model. What is being tested is
 * whether a translation on disk becomes a subtitle track; running a 583 MB translator to arrive at
 * the same file is a benchmark's job, not a suite's.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";

import { chromium } from "playwright";

import { daemon as boot } from "./daemon.mjs";
import { mirror } from "./mirror.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const video = join(HERE, "fixtures/vi-fleurs.mp4");
const problems = [];

const MODELS = ["gipformer-65m", "silero-vad-v5"];
const local = await mirror(MODELS, { name: "video" });
if (local.unreachable.length > 0) {
  for (const { id, why } of local.unreachable) console.error(`${id}: ${why}`);
  console.error("this suite imports a real recording; it needs a recogniser and a detector");
  process.exit(1);
}

const engine = await boot(process.argv, { name: "video", registry: local.registry });
const { url: appUrl, port, token, home } = engine;
const at = (path) => `${appUrl}${path}${path.includes("?") ? "&" : "?"}token=${token}`;

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

// ---- import the video ---------------------------------------------------------
const started = await (
  await fetch(at("/imports"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ path: video, model: "gipformer-65m", language: "vi" }),
  })
).json();
if (!started?.id) {
  console.error("the daemon refused the import:", JSON.stringify(started));
  process.exit(1);
}

let job = null;
for (let i = 0; i < 480; i++) {
  job = await (await fetch(at(`/imports/${started.id}`))).json();
  if (job.state === "done" || job.state === "failed") break;
  await new Promise((resolve) => setTimeout(resolve, 500));
}
if (job?.state !== "done") {
  console.error("import did not finish:", JSON.stringify(job));
  process.exit(1);
}
const meeting = job.meeting;
console.log(`imported: ${job.segments} line(s)`);

// ---- what the meeting says about itself ---------------------------------------
const detail = await (await fetch(at(`/meetings/${meeting}`))).json();
if (detail.source?.path !== video) {
  problems.push(`the meeting does not record where it came from: ${JSON.stringify(detail.source)}`);
}
if (detail.source?.video !== true) {
  problems.push("an mp4 was imported and the meeting does not think it has pictures in it");
}
if (detail.source?.available !== true) {
  problems.push("the file is exactly where it was and the daemon says it cannot be found");
}

// ---- the bytes, and seeking into them -----------------------------------------
const whole = await fetch(at(`/meetings/${meeting}/source`));
if (whole.status !== 200) {
  problems.push(`the source route answered ${whole.status}`);
}
if (!(whole.headers.get("content-type") ?? "").startsWith("video/")) {
  problems.push(`the source is served as ${whole.headers.get("content-type")}`);
}
const ranged = await fetch(at(`/meetings/${meeting}/source`), {
  headers: { Range: "bytes=0-999" },
});
if (ranged.status !== 206) {
  problems.push(`a Range request answered ${ranged.status}; the scrubber cannot seek without 206`);
}

// The lane that was a 404 for every imported meeting ever made.
const lane = await fetch(at(`/meetings/${meeting}/audio/import`));
if (lane.status !== 200) {
  problems.push(`the imported audio lane answered ${lane.status}`);
}

// ---- a translation on disk becomes a track ------------------------------------
//
// Written the way the translator writes it, so what is being tested is the reading.
const lines = detail.transcript.slice(0, 3);
// `home` is null only when a suite is pointed at a daemon somebody else started, which is a
// development convenience. Writing into a vault this process does not own would be wrong.
if (!home) {
  console.error("this suite writes a translation into the vault; run it against its own daemon");
  process.exit(1);
}
mkdirSync(join(home, "vault/translations"), { recursive: true });
writeFileSync(
  join(home, `vault/translations/${meeting}.en.md`),
  `<!-- summo:translation lang:en model:test -->\n\n` +
    lines.map((s, i) => `[00:00:0${i}] ENGLISH ${i} <!-- seq:${s.seq} -->`).join("\n") +
    "\n",
  "utf8",
);

const withTranslation = await (await fetch(at(`/meetings/${meeting}`))).json();
if (!withTranslation.subtitles?.includes("en")) {
  problems.push(
    `the meeting does not offer English subtitles: ${JSON.stringify(withTranslation.subtitles)}`,
  );
}

const both = await (await fetch(at(`/meetings/${meeting}/track/both.en.vtt`))).text();
if (!both.startsWith("WEBVTT")) {
  problems.push(
    "the bilingual track has no WEBVTT header, so a player shows nothing and says nothing",
  );
}
if (!both.includes("ENGLISH 0")) {
  problems.push("the bilingual track does not carry the translation");
}
if (!both.includes(lines[0].text)) {
  problems.push("the bilingual track does not carry what was actually said");
}

// A language nobody translated into is the original, not an error: a viewer who picks Japanese
// should see the meeting, not an empty player.
const japanese = await fetch(at(`/meetings/${meeting}/track/ja.vtt`));
if (japanese.status !== 200 || !(await japanese.text()).includes(lines[0].text)) {
  problems.push("an untranslated language should fall back to the original rather than fail");
}

// ---- the screen ----------------------------------------------------------------
const browser = await chromium.launch();
const context = await browser.newContext({
  locale: "vi-VN",
  viewport: { width: 1280, height: 900 },
});
const page = await context.newPage();
page.on("pageerror", (error) => problems.push(`pageerror: ${error.message}`));
page.on("console", (m) => m.type() === "error" && problems.push(`console: ${m.text()}`));

try {
  await page.goto(`${appUrl}?port=${port}&token=${token}#/meetings/${meeting}`, {
    waitUntil: "networkidle",
  });
  await page.waitForTimeout(2000);

  const media = await page.evaluate(() => {
    const element = document.querySelector("video") ?? document.querySelector("audio");
    return {
      tag: element?.tagName ?? null,
      tracks: [...(element?.querySelectorAll("track") ?? [])].map((t) => t.id),
    };
  });

  if (media.tag !== "VIDEO") {
    problems.push(`the meeting screen drew a ${media.tag ?? "nothing"} for a video it imported`);
  }
  for (const wanted of ["original", "en", "both.en"]) {
    if (!media.tracks.includes(wanted)) {
      problems.push(
        `the player offers no \`${wanted}\` subtitle track: ${media.tracks.join(", ")}`,
      );
    }
  }

  // Switching tracks, and asking the browser what it actually parsed. A `.vtt` a player refuses
  // looks exactly like one that works until something reads the cues back out.
  await page.getByRole("radio", { name: /\+/ }).first().click();
  let showing = null;
  for (let i = 0; i < 20 && !showing?.count; i++) {
    await page.waitForTimeout(250);
    showing = await page.evaluate(() => {
      const element = document.querySelector("video");
      const list = element?.textTracks ?? [];
      for (let i = 0; i < list.length; i += 1) {
        if (list[i].mode === "showing") {
          return {
            id: list[i].id,
            count: list[i].cues?.length ?? 0,
            first: list[i].cues?.[0]?.text ?? "",
          };
        }
      }
      return null;
    });
  }

  if (!showing) {
    problems.push("choosing a subtitle track showed none");
  } else {
    if (!showing.id.startsWith("both.")) {
      problems.push(`the bilingual track was chosen and \`${showing.id}\` is showing`);
    }
    if (showing.count === 0) {
      problems.push("the browser parsed the chosen track into zero cues");
    }
    if (!showing.first.includes("\n")) {
      problems.push(`a bilingual cue should carry two lines: ${JSON.stringify(showing.first)}`);
    }
    console.log(`showing ${showing.id}: ${showing.count} cue(s)`);
  }

  // Exactly one track visible. Two in `showing` draws both sets of cues on top of each other.
  const visible = await page.evaluate(() => {
    const list = document.querySelector("video")?.textTracks ?? [];
    let count = 0;
    for (let i = 0; i < list.length; i += 1) if (list[i].mode === "showing") count += 1;
    return count;
  });
  if (visible !== 1) {
    problems.push(`${visible} subtitle tracks are showing at once`);
  }
} finally {
  await browser.close();
  await engine.stop();
}

if (problems.length > 0) {
  for (const problem of problems) console.error(`✗ ${problem}`);
  process.exit(1);
}
console.log("video ok: imported, served with ranges, watched back with both subtitle tracks");
