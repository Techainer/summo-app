# Contributing

Thanks for looking. This file says what the project actually checks, so you can find out whether a
change is acceptable before a reviewer tells you.

## Getting it running

```bash
rustup show                     # the toolchain is pinned in rust-toolchain.toml
pnpm install
pnpm -C apps/web build          # the daemon can bundle the interface, and needs it built first
cargo run --bin summo-engine --features bundled -- --port 7788
```

Then open the address it prints. It writes a handshake to `~/.summo/engine.json` and serves the
interface itself; there is no second process to start.

`summo setup` picks models for your machine and installs them. Without it the app runs but cannot
recognise anything, and says so in a banner rather than failing quietly.

## What CI runs

Everything below runs on every pull request. Run it locally first and the review will be about the
change rather than about a comma.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

pnpm -C apps/web check          # tsc, eslint, prettier, vitest — all four
```

There are also two browser passes that are **not** in CI, because they need a browser and a built
daemon. Both start their own daemon on a vault of their own, so there is nothing to set up:

```bash
pnpm -C apps/web exec playwright install chromium
cargo build --bin summo-engine --features bundled   # the interface has to be inside the binary
pnpm -C apps/web e2e
```

`cargo test --workspace` rebuilds that same binary *without* the bundled interface, so run the build
again after a test run. The harness checks and says so rather than letting every assertion fail on a
missing header.

The screenshot pass runs at the end of `pnpm e2e`, and on its own against a vault of its own:

```bash
node apps/web/e2e/shots.mjs                                    # boots its own daemon
node apps/web/e2e/shots.mjs http://127.0.0.1:7788 7788 "$tok"  # one you are already debugging
SUMMO_LOCALE=en-US node apps/web/e2e/shots.mjs                 # in another language
```

It photographs every screen at two widths in two colour schemes and fails on a console error, on
sideways scroll, or on any text below the WCAG AA contrast ratio against the colour actually painted
behind it. It has caught more real bugs than any other check in the repository, and every one of
them had passed the unit tests — which is why it stopped being a thing you run if you remember. The
pictures land in `/tmp/shots`, and CI uploads them as an artifact when a browser job fails.

### The desktop shell

Not a workspace member — a Tauri app has its own lockfile and its own build script — so
`cargo build --workspace` does not touch it.

```bash
./scripts/sidecar.sh                     # stages the daemon where Tauri expects it
cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
pnpm -C apps/desktop dev                 # runs the sidecar script itself
```

Tauri looks for helper executables under a name ending in the target triple, which is not something
`cargo build` produces. Nothing produced it, so the shell was unbuildable from a fresh clone and
had quietly stopped compiling against Tauri v2. `tauri.conf.json` calls the script now, and CI
compiles the shell so it cannot happen again.

## Conventions

These are enforced by the commands above, not by taste:

- **Rust**: `rustfmt` with the repo's `rustfmt.toml`, and clippy with `-D warnings`. Clippy is not
  advisory here — a warning fails the build.
- **TypeScript**: strict, including `noUncheckedIndexedAccess`. ESLint runs with type information,
  which is slower and is what makes `no-floating-promises` and `switch-exhaustiveness-check`
  possible; those two catch the failures that look like the app doing nothing.
- **Formatting**: Prettier at 100 columns. Do not argue about it in review, and do not reformat a
  file you are not otherwise touching.
- **Interface text**: never written into a component. `src/i18n/*.json` holds the words and the code
  holds a key. There is a test that fails on Vietnamese in a source file, with one marked exception
  (a language listed in its own name).

### Warnings are capped, not ignored

`pnpm lint` runs with `--max-warnings 30`, which is the number there are today. Warnings cannot
grow. If you find a way to remove some, lower the number in the same commit — that is how it gets to
zero.

## Comments

The code carries a lot of them and they are held to a standard: **a comment says why, not what.** If
a line needs explaining, the explanation is the trade-off that produced it, or the bug that would
come back without it. "Increment the counter" is noise. "Compared as `YYYY-MM-DD` strings, so a
timezone cannot turn *due today* into *overdue* for anyone east of the machine that wrote it" is the
reason the line is not something simpler.

## Tests

A test should be able to fail. Before adding one, break the thing it covers and check that it goes
red — a surprising number of tests are written against the code as it happens to be rather than
against what it promises.

Name it after the promise, not the function: `a_file_a_person_wrote_by_hand_is_adopted_rather_than_skipped`
tells a reader what breaks. `test_parse_2` does not.

Fixtures should look like real input. A test for hand-written frontmatter that supplies an `id` and
a `date` is testing a file Summo itself would have written, which is the case that already worked —
that mistake shipped a bug that made notes disappear from Obsidian.

## Architecture decisions

Anything that changes the shape of the system belongs in `docs/adr/`. The existing ones are short
and argue from measurements: [0002](docs/adr/0002-no-database.md) settles why there is no database
by giving the numbers, and [0006](docs/adr/0006-organisation-without-a-database.md) says what would
reopen it. If you are proposing something an ADR rules out, the ADR names the measurement that would
change the answer — bring that.

## Commits

Present tense, and say why rather than what. The diff already shows what.

A commit that fixes a bug should say how the bug got in, if that is knowable. It is the part a
reader six months later actually needs, and it is the part that stops it happening twice.

## Reporting a bug

Whatever else you include, include **what you expected**. A transcript that is wrong and a
transcript that is right but not what you meant are different bugs with different fixes.

For anything involving your own recordings: do not attach audio. A ten-second clip you record
yourself saying the same words is enough, and does not put a colleague's voice in a public issue.

## Security

Do not open an issue. See [SECURITY.md](SECURITY.md).

### Linking faster, if you want to

Linking is the serial tail of every incremental build, and `summo-engine` with recognition links
ONNX Runtime and sherpa-onnx — which is where the seconds go. [mold](https://github.com/rui314/mold)
does that pass several times faster:

```toml
# ~/.cargo/config.toml — yours, not the project's
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

Deliberately not in the repository's `.cargo/config.toml`. `-fuse-ld=mold` is a *fatal* error on a
machine without mold — `cannot find 'ld'`, no fallback — so putting it there breaks the build for
everyone who has not installed it, which is every CI runner and every new contributor.

## Branches and releases

`master` is what ships. Nothing is pushed to it directly — including by the people who own the
repository, because a check that the owner skips is a check that stops meaning anything.

```bash
git switch -c fix/the-thing        # or feat/…, ci/…, docs/…
# … work, commit …
git push -u origin fix/the-thing   # then open a pull request
```

CI runs on every pull request and on `master`. It does **not** run on a plain branch push: a branch
with no pull request is somebody's working copy, and building it spends Windows and macOS minutes
nobody asked for. If a branch's CI matters before review, open the pull request as a draft.

Turn on branch protection for `master` in the repository settings — require the CI checks and at
least one review. Without it the rule above is a convention, and conventions are what a hurried
Friday afternoon overrides.

### Releases

A release is a tag, and only a tag:

```bash
git tag -a v0.1.0 -m "…"
git push origin v0.1.0
```

`release.yml` triggers on `v*` and on nothing else — which is why a repository with green CI and no
tags has no releases. It builds a bundle per platform on that platform (cross-compiled ONNX Runtime
and sherpa-onnx produce binaries that fail to load on the machine they were built for), publishes a
**draft**, and waits for somebody to write what changed for a user rather than shipping a list of
commit subjects.

Tag only what CI is green on. The workflow can also be started by hand from the Actions tab, which
is how to find out whether the release job works without promising anybody a release.
