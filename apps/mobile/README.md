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

## Status: the Android app runs

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
   **unsigned here**, because there is no keystore in this repository and there should not be — but
   the build can now sign, given a key. See below.

   The manifest asks for what recording needs, and one of those permissions is not obvious.
   `getUserMedia` in an Android WebView reaches `WebChromeClient.onPermissionRequest`, and wry's
   implementation requests `RECORD_AUDIO` **and `MODIFY_AUDIO_SETTINGS`** as a pair, granting the
   page only if every permission in the request came back granted. Android refuses one the manifest
   never declared, so leaving it out denies the whole request — after the user has tapped Allow.

2. **iOS is unverified.** `aarch64-apple-ios` needs Xcode, which needs macOS. Nothing here suggests
   it will be worse than Android — the same crates, the same C++ libraries, and Apple's toolchain
   is the one those libraries are best tested against — but it has not been run.

   Two declarations are in place for whoever runs it first, in `src-tauri/Info.ios.plist`, because
   `gen/apple` is generated and cannot hold them: `NSMicrophoneUsageDescription`, without which iOS
   does not refuse the microphone but kills the process, and `UIBackgroundModes: audio`, which is
   what lets a recording survive the phone locking. `minimumSystemVersion` is **14.3**, not 14.0:
   `getUserMedia` did not exist in `WKWebView` before that, so on 14.0–14.2 the app would install
   and be unable to record.

3. **Nothing has run on a *phone*.** It has now run on an emulator — API 34, x86_64, a signed
   release build — and that is where the bug below was found. What an emulator cannot answer is
   whether recognition is fast enough on a mid-range Android, what the battery cost is, or how the
   microphone behaves when a call comes in. CI runs the emulator on every push to `master` and on
   any pull request labelled `mobile`; see `scripts/android-smoke.sh`.

4. **Model size against phone memory.** `summo_models::recommend` already scores by available RAM,
   which is the right machinery. What it lacks is a measured RTF row for phone CPUs, so a ranking
   on a phone today is an extrapolation rather than a measurement. `summo-bench asr` on a device
   is what fixes that.

```bash
export ANDROID_HOME=~/Android/Sdk NDK_HOME=$ANDROID_HOME/ndk/27.2.12479018
export OPUS_STATIC=1 OPUS_LIB_DIR="$(./scripts/opus-android.sh arm64-v8a | tail -1)"
# bindgen runs its own clang, which does not inherit the cross-compiler's header paths, so
# `sherpa-rs-sys` reads the host's `/usr/include` and dies on `bits/libc-header-start.h`.
sysroot=$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$sysroot --target=aarch64-linux-android26"

pnpm -C apps/mobile exec tauri android init --skip-targets-install
pnpm -C apps/mobile exec tauri android build --apk --target aarch64
pnpm -C apps/mobile exec tauri ios build          # needs Xcode
```

`tauri android init` generates `gen/`. **`gen/android` is committed**, because the manifest in it is
hand-written and the permissions above are the difference between an app that records and one that
cannot. `gen/schemas` and `gen/apple` are not: both are regenerated and machine-specific.

Running `init` again overwrites a template file if the template has changed, which is why CI checks
the manifest still asks for a microphone between `init` and `build` rather than trusting it to.

### What running it found

The app opened, drew its interface, started its daemon — and said **"Failed to fetch"** where the
vault should have been, over a socket it was listening on itself.

A release build sets `usesCleartextTraffic="false"`, and the engine on mobile is reached at
`http://127.0.0.1:<port>`. Android refused every one of those requests. The debug build sets the
same placeholder to `"true"`, so the only configuration that worked was the one nobody ships — and
the failure is invisible to every check that does not start the app.

The fix is `res/xml/network_security_config.xml`: cleartext stays refused everywhere, with an
exception for `127.0.0.1` and `localhost`. That is not a loosening of the rule. Loopback is the
machine talking to itself, there is no network segment for anyone to sit on, and TLS there would
mean shipping a private key inside every copy of the app.

What the emulator confirmed after it:

| | |
| --- | --- |
| The interface | Draws, navigates, bottom bar and all |
| The engine | Starts in-process, listens on loopback, answers |
| Setup | Ranks models against **the device's** cores and memory, which is a live call |
| The microphone | "Allow Summo to record audio?" — granted, `RECORD_AUDIO` and `MODIFY_AUDIO_SETTINGS` both |
| Recording | Runs, times, stops cleanly |
| Models | The catalogue installs — 99 MB of Whisper tiny, downloaded, verified and stored — and the app says "Ready to record" |

And then it would not transcribe: **`configuration error: session needs a live model`**, on a machine
with a model installed. `models` was never enabled for this app, and the code that picks a model out
of the store is behind that feature, so `live_model` was never filled in. The app offered a
catalogue it could not use.

It is enabled now, per-architecture:

```toml
[target.'cfg(target_arch = "aarch64")'.dependencies]
summo-engine = { path = "…", features = ["models"] }
```

ONNX Runtime publishes no Android build for x86-64 — `ort-sys` stops with "no prebuilt binaries
available for target x86_64-linux-android" — and every phone worth shipping to is arm64. The x86-64
build exists so an emulator can be driven on a CI runner, which tests that the app starts and
reaches its engine, not that it transcribes. Feature flags are additive across those sections, so
the arm64 APK gets recognition and the emulator's does not.

The arm64 APK is **75 MB** now, against 43 MB before: a 70 MB `libsummo_mobile.so` holding the
engine, the vault, the agent, sherpa-onnx and ONNX Runtime. It links; **nothing has decoded audio
with it yet**, because this machine's emulator is x86-64 and cannot run it.

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

## Signing

The build signs when it is given a key, and produces the same unsigned `.apk` it always has when it
is not. Nothing about the key lives in this repository.

Two sources, in the order `app/build.gradle.kts` reads them:

```properties
# apps/mobile/src-tauri/gen/android/app/keystore.properties — git-ignored, for a local release
storeFile=/absolute/path/to/summo.keystore
storePassword=…
keyAlias=summo
keyPassword=…
```

```bash
# or the environment, which is what CI uses — the keystore never becomes a file in the checkout
export ANDROID_KEYSTORE=/path/to/summo.keystore ANDROID_KEYSTORE_PASSWORD=… \
       ANDROID_KEY_ALIAS=summo ANDROID_KEY_PASSWORD=…
pnpm -C apps/mobile exec tauri android build --apk --target aarch64
```

Gradle prints which of the two happened. An unsigned build says so in the same place, because an
unsigned `.apk` is not a smaller problem than a failed build — it downloads, and then the phone
refuses it, hours later, with a message about the package being corrupt.

In CI the keystore comes from `ANDROID_KEYSTORE_BASE64` and is written to the runner's temp
directory, never into the workspace. A fork has no secrets, so the step does nothing and the build
carries on unsigned — which is what makes this repository still buildable by somebody who has no key
and wants to fix a bug.

Making one, for whoever does this first:

```bash
keytool -genkey -v -keystore summo.keystore -alias summo \
        -keyalg RSA -keysize 4096 -validity 10000
```

**Keep it, and keep it backed up.** Android identifies an app by its signature: a Play listing
updated with a different key is refused, and there is no recovery that does not involve a new
listing and every user reinstalling.
