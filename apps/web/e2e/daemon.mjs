/**
 * A daemon and a vault, for a browser test to drive.
 *
 * Every suite in this folder used to take `<url> <port> <token>` on the command line, which meant
 * they only ran against a daemon somebody had already started, pointed at a vault somebody had
 * already filled. Nothing said so. Run them against a fresh vault — which is what anybody following
 * `package.json` would do — and all seven failed on the first-run overlay, because a vault with
 * nothing in it is a vault the app quite correctly asks you to set up first.
 *
 * So the suites boot their own. Each one gets a vault of its own under `/tmp`, seeded with the
 * meetings it needs and marked as past onboarding, and the daemon is stopped at the end.
 *
 * The command-line form still works, for pointing a suite at a daemon you are already debugging.
 */
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

/**
 * The registry checkout.
 *
 * Beside this repository by default, which is where a developer clones it. On CI it cannot be:
 * `actions/checkout` refuses a path outside the workspace, so the job clones it *inside* and points
 * here with `SUMMO_REGISTRY_DIR`. The old line was a fixed `../../../../summo-registry` and the
 * browser job spent its life failing on "Repository path is not under GITHUB_WORKSPACE".
 */
const REGISTRY = process.env.SUMMO_REGISTRY_DIR ?? join(HERE, "../../../../summo-registry");

/** The daemon, built with the interface inside it. `--features bundled` is what puts it there. */
const BINARY = join(HERE, "../../../target/debug/summo-engine");

/**
 * Marks setup as done, so the app shows itself rather than the checklist.
 *
 * Matches `DONE_FILE` in `crates/summo-engine/src/onboarding.rs`. Written rather than clicked
 * through: the checklist is `first-run.mjs`'s subject, and every other suite has to get past it.
 */
const ONBOARDED = "onboarded";

/**
 * Two meetings, a note, four tasks and a draft — enough for every suite here to have something to
 * drive. The specifics are load-bearing and the suites assert them:
 *
 * - Two owners, so filtering by one can visibly narrow a column. With a single owner the filter
 *   was correct and looked broken.
 * - Indented checkboxes under the agent task, which is how it records its own steps.
 * - A `<!-- summo:draft -->` heading, since an unapproved summary is what the draft panel is for.
 * - A note with no `id` and no `date`, which is what Obsidian leaves behind.
 * - Transcript lines attributed to `S2` and `S3`, matching the voice logs, because that is what a
 *   recording looks like before anybody has said who was talking — and it is the state the naming
 *   affordance in the transcript exists for.
 */
/**
 * A day, `n` days before today, as `YYYY-MM-DD`.
 *
 * The seeded meetings used to carry fixed dates in August 2026. They were inside every window the
 * app asks about when they were written and they aged out of them overnight: one morning the
 * analytics screen's default range held nothing, and the suite that checks no screen is mostly
 * background failed on a screen that had correctly gone empty. A fixture that stops exercising the
 * thing it was written for is worse than one that fails — it goes on passing.
 *
 * Relative, therefore. The two recordings are always yesterday and the day before, which is what
 * "this week" means on any day somebody runs this.
 */
function daysAgo(n) {
  const day = new Date();
  day.setDate(day.getDate() - n);
  return day.toISOString().slice(0, 10);
}

/** Yesterday and the day before, exported so a suite can name the files this wrote. */
export const RECENT = daysAgo(1);
export const EARLIER = daysAgo(2);

function seedVault(home) {
  const meetings = join(home, "vault/meetings");
  const notes = join(home, "vault/notes");
  mkdirSync(join(meetings, "khach-hang"), { recursive: true });
  mkdirSync(notes, { recursive: true });

  writeFileSync(
    join(meetings, `${RECENT}-hop-dau-tuan.md`),
    `---
id: 01E2E0
date: ${RECENT}T10:00:00+07:00
duration: 2538
participants: ["[[Bạn]]", "[[Ngọc]]"]
tags: [weekly, sản-phẩm]
color: teal
---

# Họp đầu tuần

## Tóm tắt <!-- summo:draft -->
Chốt ngân sách quý bốn.

## Việc cần làm
- [ ] @Ngọc Chốt spec API <!-- id:01T1 due:2026-08-20 status:todo -->
- [ ] @Bình Gửi báo giá <!-- id:01T3 status:todo -->
- [ ] @agent Tạo lịch cho mốc ra mắt <!-- id:01T2 status:todo -->
  - [x] Quét ghi chú tìm mốc thời gian
  - [ ] Soạn sự kiện lịch

## Transcript
**[00:12:04] Bạn** — Mình họp về ngân sách nhé <!-- seq:0 end:734.0 -->
**[00:13:10] Ngọc** — Em nghĩ nên chốt spec trước <!-- seq:1 end:795.0 -->
**[00:14:02] S2** — Vậy chốt hôm nay nhé <!-- seq:2 end:848.0 -->
**[00:15:30] S3** — Em gửi bản nháp chiều nay <!-- seq:3 end:935.0 -->
`,
  );

  // Bytes, not a real recording. The player renders from the *presence* of a lane, and the route
  // serves whatever is on disk — the daemon's own tests seed audio the same way. Decoding it is
  // not what any of these suites are about.
  const audio = join(home, "audio/01E2E0");
  mkdirSync(audio, { recursive: true });
  writeFileSync(join(audio, "mic.opus"), Buffer.alloc(4096, 9));

  writeFileSync(
    join(meetings, `khach-hang/${EARLIER}-demo.md`),
    `---
id: 01E2E1
date: ${EARLIER}T09:00:00+07:00
duration: 1800
participants: ["[[Bạn]]", "[[Bình]]"]
tags: [khách-hàng]
color: red
---

# Demo khách hàng ACME

## Tóm tắt
Họ muốn bản dùng thử.

## Transcript
**[00:03:00] S2** — Bên mình cần bản dùng thử trước <!-- seq:0 end:186.0 -->
`,
  );

  // No `id` and no `date`, which is what somebody typing in Obsidian actually leaves behind.
  writeFileSync(
    join(notes, "y-tuong-gia.md"),
    "---\ntags: [sản-phẩm]\n---\n\n# Ý tưởng giá\n\nBán 3–4 đô một tháng.\n",
  );

  // A voice nobody has named, so the voice book has its question on it.
  //
  // JSON rather than the binary `.vec` of ADR 0003: `VoiceLog::load` dispatches on the file's magic
  // rather than on its extension — a half-finished migration leaves JSON in a `.vec` — so this is a
  // supported shape and not a trick. Writing the binary one from here would mean a second
  // implementation of that format in a language that does not have to have one.
  const logs = join(home, "voices/meetings");
  mkdirSync(logs, { recursive: true });
  const log = (meeting, samples) =>
    writeFileSync(
      join(logs, `${meeting}.vec`),
      JSON.stringify({
        meeting,
        schema: 1,
        model: "campplus-sv",
        samples: samples.map((sample) => ({ confirmed: false, ...sample })),
      }),
    );
  // Both recordings, because both were recorded: a vault where only one meeting has vectors is a
  // vault that could not have happened, and the screens that read them were being measured against
  // it.
  //
  // `seq` matches the `<!-- seq:n -->` on the transcript line it came from. That is the join:
  // naming a voice rewrites the utterances whose sequence numbers its samples carry, so a log whose
  // numbering does not line up produces a name nobody ever sees applied.
  log("01E2E0", [
    { seq: 2, t0: 842, duration: 6, label: "S2", embedding: [0, 1, 0, 0] },
    { seq: 3, t0: 930, duration: 5, label: "S3", embedding: [0, 0, 1, 0] },
  ]);
  log("01E2E1", [{ seq: 0, t0: 180, duration: 6, label: "S2", embedding: [1, 0, 0, 0] }]);
}

/**
 * Start a daemon on a vault of its own.
 *
 * `port: 0` asks the operating system for a free one, so two suites running at once cannot collide
 * — which is what a fixed port did the first time this was tried in parallel.
 */
/**
 * Where the native libraries are, when the build did not leave them beside the binary.
 *
 * A build with `--features models` links sherpa-onnx, and the binary is linked with an `$ORIGIN`
 * rpath — right for the shipped bundle, where the libraries sit beside the executable. Out of
 * `target/debug` that only works because Cargo copies them there while the build script *runs*; on
 * a machine with a warm cache it does not run, the copy never happens, and the daemon dies with
 * "libsherpa-onnx-c-api.so: cannot open shared object file". Cargo's own `deps/` always has them,
 * so the harness points at it: the failure has nothing to do with whatever is being tested, which
 * is the worst kind of red build.
 */
function libraries() {
  const beside = dirname(BINARY);
  const key = process.platform === "darwin" ? "DYLD_LIBRARY_PATH" : "LD_LIBRARY_PATH";
  return {
    [key]: [beside, join(beside, "deps"), process.env[key]].filter(Boolean).join(":"),
  };
}

export async function boot({ name = "e2e", seed = true, registry = REGISTRY } = {}) {
  const home = join("/tmp", `summo-${name}-${process.pid}`);
  rmSync(home, { recursive: true, force: true });
  mkdirSync(home, { recursive: true });
  writeFileSync(join(home, ONBOARDED), "");
  if (seed) seedVault(home);

  const child = spawn(BINARY, ["--home", home, "--port", "0"], {
    stdio: "pipe",
    // The registry the catalogue reads from. Pointed at the checkout beside this one so the suite
    // tests a real registry without depending on a deployed CDN — and so it keeps passing when the
    // network is not there. A caller can substitute one: `models.mjs` builds a registry whose file
    // URLs point at a local server, so installing does not reach the public internet either.
    env: { ...process.env, SUMMO_REGISTRY: registry, ...libraries() },
  });
  // Detached from Node's own exit accounting. A suite that forgets `stop()` should end with a
  // failed assertion, not hang until whatever is running it gives up — which is how a passing
  // suite came back as a 124.
  child.unref();
  const log = [];
  child.stdout.on("data", (d) => log.push(String(d)));
  child.stderr.on("data", (d) => log.push(String(d)));
  // `unref` on the child is not enough: a pipe with a `data` handler is itself a live handle, so a
  // suite that forgot `stop()` ran to completion, printed its result, and then hung forever. It was
  // the last suite in `pnpm e2e` that did it, so the whole chain never returned — and the symptom
  // is a run that looks like it is still working rather than one that failed.
  child.stdout.unref();
  child.stderr.unref();

  // Killed when this process ends, however it ends.
  //
  // `stop()` at the bottom of a suite covers the run that passes. The run that throws — a failed
  // assertion, a locator that never appeared, `process.exit(1)` on the first problem — never
  // reaches it, and the daemon it started is `unref`'d, so Node exits and leaves it running. This
  // machine had sixty-eight of them, the oldest two days old, each holding a port and a vault under
  // `/tmp` that nothing would ever delete.
  //
  // `exit` handlers must be synchronous, which `kill` and `rmSync` both are. SIGINT and SIGTERM get
  // their own, because a handler on either replaces the default that would have ended the process,
  // so re-raising is how the exit code stays honest.
  // The vault outlives a failure on purpose: a suite that fails on "the note never saved" is a
  // suite whose vault is the evidence. It goes on a clean exit and stays on a dirty one, with the
  // path printed so it can be looked at and deleted.
  let stopped = false;
  const cleanup = (code) => {
    if (stopped) return;
    stopped = true;
    try {
      child.kill("SIGKILL");
    } catch {
      // Already gone, which is the outcome being asked for.
    }
    if (code === 0) rmSync(home, { recursive: true, force: true });
    else console.error(`the vault this ran against is at ${home}`);
  };
  process.once("exit", cleanup);
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.once(signal, () => {
      cleanup(1);
      process.kill(process.pid, signal);
    });
  }

  const handshakeFile = join(home, "engine.json");
  const deadline = Date.now() + 20_000;

  /**
   * Wait for the handshake, and nothing else.
   *
   * Deliberately the only thing inside the `try`. The checks below used to live in here too, and a
   * `catch` written for "the file is not there yet" swallowed them whole — so a daemon serving no
   * interface reported "did not come up in 20s", and the sentence naming the actual cause was
   * built, thrown, and discarded on every single run. The diagnostics that matter are the ones a
   * developer sees at four in the afternoon with a suite failing for the wrong reason.
   */
  const handshake = await (async () => {
    for (;;) {
      try {
        const found = JSON.parse(readFileSync(handshakeFile, "utf8"));
        if (found.port > 0 && found.token) return found;
      } catch {
        // Not written yet, which is the normal state for the first second.
      }
      if (Date.now() > deadline) {
        child.kill();
        throw new Error(
          `the daemon did not come up in 20s. Is it built?\n` +
            `  cargo build --bin summo-engine --features bundled\n` +
            log.join(""),
        );
      }
      await new Promise((r) => setTimeout(r, 100));
    }
  })();

  const refuse = (why) => {
    child.kill();
    throw new Error(why);
  };

  // The interface has to be *inside* the binary, and `cargo test --workspace` rebuilds this same
  // path without the feature that puts it there — so a suite run after a test run gets a daemon
  // that answers the API and serves no app. Every assertion then fails on a missing `header`,
  // which is a long way from the cause.
  const page = await fetch(`http://127.0.0.1:${handshake.port}/`).catch(() => null);
  const document = page ? await page.text() : "";
  if (!document.includes('<div id="root">')) {
    refuse(
      "the daemon is serving no interface. Rebuild it with the feature that bundles one:\n" +
        "  cargo build --bin summo-engine --features bundled",
    );
  }

  // And it has to be *this* interface.
  //
  // `bundled` bakes `dist/` into the binary at compile time, so a web change followed by
  // `pnpm build` alone leaves the daemon serving whatever was on disk when the binary was last
  // linked. Every suite then passes against yesterday's app — which is worse than failing, because
  // the run reports that the change works. It is not hypothetical: it is how a check added to this
  // very file was first observed to pass.
  //
  // Vite names the entry script after a hash of its contents, so comparing that one filename is an
  // exact answer: same name, same bundle.
  const entry = (html) => html.match(/assets\/(index-[A-Za-z0-9_-]+\.js)/)?.[1];
  const built = entry(readFileSync(join(HERE, "..", "dist", "index.html"), "utf8"));
  if (built && entry(document) !== built) {
    refuse(
      `the daemon is serving an older interface than the one in dist/.\n` +
        `  serving ${entry(document)}\n  built   ${built}\n` +
        "Rebuild the binary after building the web app — the interface is compiled into it:\n" +
        "  pnpm -C apps/web build && cargo build --bin summo-engine --features bundled",
    );
  }

  return {
    home,
    port: handshake.port,
    token: handshake.token,
    /// Everything the daemon has printed, for a suite whose failure is on that side of the socket
    /// rather than in the browser.
    log: () => log.join(""),
    url: `http://127.0.0.1:${handshake.port}`,
    stop() {
      // Through the same path as the exit handler, so a suite that calls it and a suite that dies
      // before it get the same treatment — and so calling it twice is harmless.
      cleanup(0);
    },
  };
}

/**
 * The daemon a suite should use: the one named on the command line, or one it starts itself.
 *
 * `stop()` is a no-op for a daemon somebody else started — a test suite should not kill a process
 * it did not launch, and deleting the vault it was pointed at would be worse still.
 */
export async function daemon(argv, options = {}) {
  const [url, port, token] = argv.slice(2);
  if (url && token) {
    return { url, port: Number(port), token, home: null, stop() {} };
  }
  return boot(options);
}
