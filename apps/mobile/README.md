# Summo on iOS and Android

Same interface as the desktop app — `apps/web` is the frontend for both — over the same engine.

## What is different, and why

**The engine is linked in, not spawned.** iOS does not let an App Store build run another
executable: no `fork` it may use, no second binary the system will launch, no sidecar. Android
allows it and punishes it — a second process is the first thing killed under memory pressure, which
on a phone is most of the time.

So `summo_engine::embedded` starts the daemon inside the app process and the webview talks to it
over loopback exactly as it would to a sidecar: same routes, same token, same protocol. The webview
asks for the port and token with a `handshake` command instead of reading `engine.json`.

The cost is crash isolation. On desktop a panic in a decoder kills the daemon and the app reconnects
to the transcript already on disk. Here it takes the app with it, which is why the recorder flushes
on its own interval — a crash costs seconds of a meeting rather than the meeting.

**The vault lives in the app's sandbox.** `~/.summo` does not exist on either platform. Anything
written outside the directory the OS hands the app is removed or refused.

## What works and what does not

|                      |                                                                                                                                                              |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Microphone recording | Planned, both platforms. Covers in-person meetings.                                                                                                          |
| In-app meeting audio | Android only, via `AudioPlaybackCapture`. An app may opt out of being captured, and the meeting apps are exactly the ones that might. iOS has no equivalent. |
| **Call recording**   | **Not implementable.** See below.                                                                                                                            |

Call recording is out permanently, not "later". iOS has never exposed call audio to third-party
apps, and iOS 18.1's own call recording ships with no third-party API. Google Play has banned
third-party call recording apps since May 2022 — only the OEM dialer may. This is policy and
platform, not difficulty.

## Status: the Android app assembles

**The entire workspace type-checks for `aarch64-linux-android`** — the engine with its bundled
interface, the vault, the agent, the model registry, and `summo-asr` with sherpa-onnx and ONNX
Runtime behind it. Checked with NDK r27c, and CI checks it on every pull request so it cannot
quietly stop being true.

That is further than this file used to claim. It said the native runtimes were "real work that has
not been attempted", and the answer turned out to be that the dependencies already handle it: the
build scripts pick up the NDK's clang and cross-compile without any help from us.

What has **not** happened:

1. **An `.apk` is produced; no `.ipa` is.** The Android app builds — 42 MB, one 40 MB
   `libsummo_mobile.so` holding the engine, the vault, the agent, sherpa-onnx and ONNX Runtime —
   and CI assembles it on every push to `master` and on any pull request labelled `mobile`. It is
   **unsigned**: there is no keystore in this repository and there should not be. Still to do
   before anybody can install it usefully: signing, and the manifest's `RECORD_AUDIO` and
   foreground-service declarations.

2. **iOS is unverified.** `aarch64-apple-ios` needs Xcode, which needs macOS. Nothing here suggests
   it will be worse than Android — the same crates, the same C++ libraries, and Apple's toolchain
   is the one those libraries are best tested against — but it has not been run.

3. **Nothing has run on a phone.** Compiling says the code is well-formed for the platform. It says
   nothing about whether the microphone permission flow works, whether recognition is fast enough
   on a mid-range Android, or what the battery cost is.

4. **Model size against phone memory.** `summo_models::recommend` already scores by available RAM,
   which is the right machinery. What it lacks is a measured RTF row for phone CPUs, so a ranking
   on a phone today is an extrapolation rather than a measurement. `summo-bench asr` on a device
   is what fixes that.

```bash
export ANDROID_HOME=~/Android/Sdk NDK_HOME=$ANDROID_HOME/ndk/27.2.12479018
export OPUS_STATIC=1 OPUS_LIB_DIR="$(./scripts/opus-android.sh arm64-v8a | tail -1)"

pnpm -C apps/mobile exec tauri android init --skip-targets-install
pnpm -C apps/mobile exec tauri android build --apk --target aarch64
pnpm -C apps/mobile exec tauri ios build          # needs Xcode
```

`tauri android init` generates `gen/`; it is not committed because it is machine-specific and
regenerable.

### Three things that had to be fixed before any of that ran

Every one of them was invisible to `cargo check --target aarch64-linux-android`, which is what CI
had been running and calling portability.

**Opus does not cross-compile on its own.** `audiopus_sys` 0.1.8 runs `configure` with no `--host`,
so autoconf believes it is building for the machine it is running on: it compiles a test program
with the NDK's compiler, tries to _run_ it, and stops. Set the compiler without the flag and it goes
one step further and produces an x86-64 `libopus.so`, which the linker rejects as "incompatible with
aarch64linux". `scripts/opus-android.sh` builds the upstream release properly and `OPUS_LIB_DIR` —
the crate's own supported escape hatch — points at it.

**`use tauri::Manager` is not enough in Tauri v2.** `emit()` moved to a separate `Emitter` trait, so
the two lines that tell the interface the engine is up did not compile. A type check for the target
never reached them, because it never reached this crate.

**Gradle could not find the Tauri CLI.** The generated `rustBuildArm64Release` task runs
`pnpm tauri …` with the working directory set to `src-tauri`, which has no `package.json` — so pnpm
walked up, found no `tauri` binary, and failed with a message about a recursive exec. The CLI is a
root dev dependency now, which is where a tool used by two apps belongs.
