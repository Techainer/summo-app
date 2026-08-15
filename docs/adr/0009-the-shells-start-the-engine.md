# 0009 — The shells start the engine, and say so when they cannot

**Status:** accepted
**Date:** 2026-08-15

## The decision

The desktop and mobile shells are responsible for there being an engine, and for telling the
interface where it is. Both answer one command, `engine_handshake`, with one of three states —
`starting`, `ready`, `failed` — and the interface renders nothing but a splash until it is not
`starting`.

## What was actually wrong

Neither shell did any of this, and neither had ever been run.

- **The desktop app never started its daemon.** `tauri.conf.json` has carried `externalBin` from the
  beginning, `scripts/sidecar.sh` stages the binary into the bundle, and CI compiles the shell on
  every pull request. No line of the shell ever spawned it.
- **Neither shell told the interface anything.** The mobile shell exposed a `handshake` command that
  nothing called. `lib/session.ts` documented a query string "for the Tauri webview" that nothing
  wrote.
- So both apps fell back to `DEV_HANDSHAKE` — `{ port: 8710, token: "" }` — and connected to
  nothing, unless a developer happened to be running an unauthenticated daemon on that port.
- **The tray icon and the global shortcut did nothing.** The shell emitted the Tauri event
  `summo://toggle-record`; the interface listened for the DOM event `summo:toggle-record`. Nothing
  translated between them, so the promise the tray exists for — recording starts in under a second,
  without focusing the app — was not kept by either half.
- **The bundled daemon could not have started anyway.** It is linked against ONNX Runtime and
  sherpa-onnx with a runpath of `$ORIGIN`, and only the executable was staged. It exits 127.

Every one of these is invisible to the twenty browser suites, and for the same reason: there the
daemon serves the interface itself, injects the handshake into the document, and is the page's own
origin. The suites test the product; none of them tests the *app*.

## Why a command, and not the URL

The obvious alternative is to point the window at `http://127.0.0.1:<port>/?token=…`, which the
daemon can serve because it already carries the interface. It was rejected:

- A page on a remote scheme has no privilege in Tauri unless a capability names it, so the file
  dialog would have to be granted to *any* page on loopback.
- The mobile shell cannot do it at all: its webview is created before the embedded engine has
  finished starting, so there is no port to put in a URL yet.

A command works on both, and keeps the window on the bundle's own scheme.

## Adopt before spawn

`summo serve` and the app keep their state in the same place, and two daemons writing one vault
corrupts an index rather than slowing anything down. So an `engine.json` whose port still accepts a
connection is adopted as it is, and a sidecar is spawned only when nothing answers. A daemon the app
adopted is left running when the app quits; one it started is killed with it.

A connection, not a request: the daemon refuses an unauthenticated caller, so a 401 and a healthy
daemon look identical from outside, and the only question being asked is "is it gone".

## Failing out loud

A failure is a state, not a timeout. Whatever the daemon printed before dying is captured and
becomes the message — "cannot bind loopback port: Address already in use" is something a user can
act on; "could not start" is something they can only report. It is shown on the splash verbatim and
printed to stderr, because someone running the binary from a terminal to find out why it will not
work should not have to read it off a screen.

The splash has **no translated words**. It renders above the i18n provider, which reads the language
from the daemon that is not up yet — so it shows the mark, and the failure in the daemon's own
words. A sentence in the wrong language is worse than none.

## Consequences

- The daemon's output is drained for the life of the process. It logs a line per request, and a
  pipe nobody reads fills at 64 kB and blocks the writer: an app that works for a minute and then
  stops.
- `apps/web/src/lib/csp.test.ts` asserts every directive the interface relies on names the daemon.
  `connect-src` was the only one that did, which is why `fetch` worked in the packaged apps while
  every picture in a note and every recording played back was blocked.
- The libraries are staged into `binaries/lib/` and shipped as bundle resources, and the shell puts
  that directory on the daemon's library path. Untested on a signed macOS build, where the hardened
  runtime strips `DYLD_*` from a child and the dylibs will have to sit beside the sidecar instead.
- A development run passes `--dev` to the daemon it spawns, because the window is served by Vite on
  another loopback port and the daemon refuses an origin it does not recognise. A packaged app does
  not: its page is `tauri://localhost`, which is trusted by scheme, and `--dev` would trust every
  port on the machine.
