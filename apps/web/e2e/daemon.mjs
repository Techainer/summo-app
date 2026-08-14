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
 */
function seedVault(home) {
  const meetings = join(home, "vault/meetings");
  const notes = join(home, "vault/notes");
  mkdirSync(join(meetings, "khach-hang"), { recursive: true });
  mkdirSync(notes, { recursive: true });

  writeFileSync(
    join(meetings, "2026-08-10-hop-dau-tuan.md"),
    `---
id: 01E2E0
date: 2026-08-10T10:00:00+07:00
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
**[00:14:02] Bạn** — Vậy chốt hôm nay nhé <!-- seq:2 end:848.0 -->
**[00:15:30] Ngọc** — Em gửi bản nháp chiều nay <!-- seq:3 end:935.0 -->
`,
  );

  // Bytes, not a real recording. The player renders from the *presence* of a lane, and the route
  // serves whatever is on disk — the daemon's own tests seed audio the same way. Decoding it is
  // not what any of these suites are about.
  const audio = join(home, "audio/01E2E0");
  mkdirSync(audio, { recursive: true });
  writeFileSync(join(audio, "mic.opus"), Buffer.alloc(4096, 9));

  writeFileSync(
    join(meetings, "khach-hang/2026-08-09-demo.md"),
    `---
id: 01E2E1
date: 2026-08-09T09:00:00+07:00
duration: 1800
participants: ["[[Bạn]]", "[[Bình]]"]
tags: [khách-hàng]
color: red
---

# Demo khách hàng ACME

## Tóm tắt
Họ muốn bản dùng thử.

## Transcript
**[00:03:00] Bình** — Bên mình cần bản dùng thử trước <!-- seq:0 end:186.0 -->
`,
  );

  // No `id` and no `date`, which is what somebody typing in Obsidian actually leaves behind.
  writeFileSync(
    join(notes, "y-tuong-gia.md"),
    "---\ntags: [sản-phẩm]\n---\n\n# Ý tưởng giá\n\nBán 3–4 đô một tháng.\n",
  );
}

/**
 * Start a daemon on a vault of its own.
 *
 * `port: 0` asks the operating system for a free one, so two suites running at once cannot collide
 * — which is what a fixed port did the first time this was tried in parallel.
 */
export /**
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

async function boot({ name = "e2e", seed = true, registry = REGISTRY } = {}) {
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

  const handshakeFile = join(home, "engine.json");
  const deadline = Date.now() + 20_000;
  for (;;) {
    try {
      const handshake = JSON.parse(readFileSync(handshakeFile, "utf8"));
      if (handshake.port > 0 && handshake.token) {
        // The interface has to be *inside* the binary, and `cargo test --workspace` rebuilds this
        // same path without the feature that puts it there — so a suite run after a test run gets
        // a daemon that answers the API and serves no app. Every assertion then fails on a missing
        // `header`, which is a long way from the cause.
        const page = await fetch(`http://127.0.0.1:${handshake.port}/`).catch(() => null);
        if (!page || !(await page.text()).includes('<div id="root">')) {
          child.kill();
          throw new Error(
            "the daemon is serving no interface. Rebuild it with the feature that bundles one:\n" +
              "  cargo build --bin summo-engine --features bundled",
          );
        }
        return {
          home,
          port: handshake.port,
          token: handshake.token,
          /// Everything the daemon has printed, for a suite whose failure is on that side of the
          /// socket rather than in the browser.
          log: () => log.join(""),
          url: `http://127.0.0.1:${handshake.port}`,
          stop() {
            child.kill();
            rmSync(home, { recursive: true, force: true });
          },
        };
      }
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
