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

| | |
|---|---|
| Microphone recording | Planned, both platforms. Covers in-person meetings. |
| In-app meeting audio | Android only, via `AudioPlaybackCapture`. An app may opt out of being captured, and the meeting apps are exactly the ones that might. iOS has no equivalent. |
| **Call recording** | **Not implementable.** See below. |

Call recording is out permanently, not "later". iOS has never exposed call audio to third-party
apps, and iOS 18.1's own call recording ships with no third-party API. Google Play has banned
third-party call recording apps since May 2022 — only the OEM dialer may. This is policy and
platform, not difficulty.

## Status: scaffolded, not built

Everything in `src-tauri/` is written but **has never been compiled**, because building it needs
Xcode and the Android NDK, which this repository's CI does not have and which were not available
where it was written. Treat it as a starting point that reflects the architecture, not as a working
build.

Two things are known to be unresolved:

1. **Native model runtimes.** `summo-engine`'s `models` feature links sherpa-onnx and ONNX Runtime.
   Cross-compiling both for `aarch64-apple-ios` and `aarch64-linux-android` is real work that has
   not been attempted here. Until it is, this builds only without that feature — which means the
   app runs, shows the vault, and cannot transcribe.

2. **Model size against phone memory.** `summo_models::recommend` already scores by available RAM,
   which is the right machinery. What it lacks is a measured RTF row for phone CPUs, so a ranking
   on a phone today is an extrapolation rather than a measurement. `summo-bench asr` on a device
   is what fixes that.

```bash
pnpm --filter @summo/mobile android      # needs the Android SDK and NDK
pnpm --filter @summo/mobile ios          # needs Xcode
```

`tauri android init` / `tauri ios init` generate the `gen/` directory these commands expect; it is
not committed because it is machine-specific and regenerable.
