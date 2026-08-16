#!/usr/bin/env bash
#
# Start the packaged desktop app and prove it is running.
#
#   ./scripts/smoke-desktop.sh apps/desktop/src-tauri/target/release/bundle/appimage/Summo_*.AppImage
#   ./scripts/smoke-desktop.sh /tmp/deb-root/usr/bin/summo-desktop
#   ./scripts/smoke-desktop.sh "apps/desktop/src-tauri/target/release/bundle/macos/Summo.app"
#
# The `desktop-bundle` job builds the installers and checks what is inside them. That is a different
# claim from "it starts": every bug this project has had in packaging looked identical from outside
# — an app that installs, opens a window, and shows nothing, because the daemon it is supposed to
# spawn died on a missing library one line into its life. The bundle contents check would not have
# caught the rpath that made `linuxdeploy` fail, and it would not catch a sidecar that is present,
# executable, and immediately exits.
#
# So this runs the thing. It is deliberately small: what it asserts is that the app spawns its
# daemon, that the daemon answers, that it serves the interface, and that both are still alive a
# few seconds later — the four things that were false at some point this month, none of which any
# other check in the repository can see.
#
# Not asserted: anything on the screen. Reading pixels out of a webview needs a screenshot tool, a
# window manager and a tolerance for antialiasing, and the browser suites already photograph every
# screen at two widths in two colour schemes. What they cannot do is start the app.

set -euo pipefail

APP="${1:?usage: smoke-desktop.sh <path to the app binary, AppImage or .app bundle>}"

# A macOS bundle is a directory. The executable inside it is what actually runs, and running it
# directly rather than through `open` is what lets this script give it a vault of its own: `open`
# hands the app to `launchd`, which does not carry this shell's environment, so `SUMMO_HOME` would
# be dropped and the check would run against the developer's own vault.
if [[ -d "${APP}" && "${APP}" == *.app ]]; then
  BUNDLE="${APP}"
  # Whatever is in there, rather than the bundle's own name: the bundle is `Summo.app` and the
  # executable inside it is the Cargo binary, `summo-desktop`. Deriving one from the other looked
  # right and reported "not executable" against a path that had never existed.
  APP="$(find "${BUNDLE}/Contents/MacOS" -maxdepth 1 -type f -perm -u+x | head -1)"
  [[ -n "${APP}" ]] || {
    echo "no executable inside ${BUNDLE}/Contents/MacOS:" >&2
    ls -l "${BUNDLE}/Contents/MacOS" >&2
    exit 2
  }
fi

[[ -x "${APP}" ]] || {
  echo "not executable: ${APP}" >&2
  exit 2
}

# The signature, on macOS only.
#
# An unsigned arm64 binary is not "unverified" — it is refused. Apple Silicon requires at least an
# ad-hoc signature, and a user who downloads one is told the app "is damaged and can't be opened",
# which is what a person actually saw. The bundle is ad-hoc signed at build time
# (`signingIdentity: "-"` in `tauri.conf.json`); this fails the build if that ever stops being true,
# because the failure is invisible on the machine that produces it and total on the machine that
# receives it.
if [[ "$(uname -s)" == "Darwin" && -n "${BUNDLE:-}" ]]; then
  codesign --verify --deep --strict --verbose=2 "${BUNDLE}" ||
    { echo "smoke: the bundle carries no valid signature — macOS will call it damaged" >&2; exit 1; }
  echo "signature: $(codesign -dv "${BUNDLE}" 2>&1 | grep -i "^Signature\|^Identifier" | tr "\n" " ")"
fi

# A vault of its own, so this never touches ~/.summo — and so the handshake it waits for cannot be
# one a developer's own daemon left behind. `engine.rs` reads `SUMMO_HOME` before anything else for
# exactly this.
HOME_DIR="$(mktemp -d /tmp/summo-smoke-XXXXXX)"
# The daemon is a native program, and on Windows this script runs under Git Bash: arguments are
# path-translated for it, environment variables are not. A Windows binary handed `/tmp/summo-smoke-x`
# as its home would create a directory called `tmp` beside itself and this script would wait forever
# for a handshake in the other one.
if [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* || "$(uname -s)" == CYGWIN* ]]; then
  export SUMMO_HOME="$(cygpath -w "${HOME_DIR}")"
else
  export SUMMO_HOME="${HOME_DIR}"
fi
# The AppImage runtime needs FUSE, which no CI runner has any more. Extracting instead is the
# documented way out and costs a second.
export APPIMAGE_EXTRACT_AND_RUN=1

LOG="${HOME_DIR}/app.log"
FAILED=0

cleanup() {
  # The window first, then anything it started. A daemon that outlives this script is the leak the
  # browser harness had, and there is no reason to reproduce it here.
  [[ -n "${APP_PID:-}" ]] && kill "${APP_PID}" 2>/dev/null || true
  sleep 1
  [[ -n "${APP_PID:-}" ]] && kill -9 "${APP_PID}" 2>/dev/null || true
  pkill -f "summo-engine --home ${HOME_DIR}" 2>/dev/null || true
  if [[ "${FAILED}" -eq 0 ]]; then
    rm -rf "${HOME_DIR}"
  else
    echo "--- what the app printed ---"
    cat "${LOG}" 2>/dev/null || true
    echo "the run is at ${HOME_DIR}"
  fi
}
trap cleanup EXIT

fail() {
  FAILED=1
  echo "smoke: $1" >&2
  exit 1
}

# A display, because a Tauri app with no `DISPLAY` exits before it reaches any of the code being
# tested — and its complaint is about GTK, which is a long way from what is being checked here.
# macOS and Windows have a window server already; `xvfb-run` exists on neither and is needed on
# neither.
echo "starting ${APP##*/}"
if [[ "$(uname -s)" != "Linux" ]]; then
  "${APP}" > "${LOG}" 2>&1 &
else
  xvfb-run -a --server-args="-screen 0 1280x900x24" "${APP}" > "${LOG}" 2>&1 &
fi
APP_PID=$!

# The handshake, which is the app saying it has a daemon and where it is. Sixty seconds because a
# cold start on a loaded CI runner loads the vault, the settings and the model manifests first.
HANDSHAKE="${HOME_DIR}/engine.json"
for _ in $(seq 1 120); do
  [[ -s "${HANDSHAKE}" ]] && break
  kill -0 "${APP_PID}" 2>/dev/null || fail "the app exited before it wrote a handshake"
  sleep 0.5
done
[[ -s "${HANDSHAKE}" ]] || fail "no handshake after 60s — the app never started its daemon"

PORT="$(grep -o '"port"[[:space:]]*:[[:space:]]*[0-9]*' "${HANDSHAKE}" | grep -o '[0-9]*$')"
[[ -n "${PORT}" ]] || fail "the handshake has no port in it: $(cat "${HANDSHAKE}")"
echo "handshake: port ${PORT}"

# It answers. `/health` needs no token — that is asserted in the daemon's own tests — so this is a
# check of the process and not of the credential.
curl -fsS --max-time 10 "http://127.0.0.1:${PORT}/health" > /dev/null ||
  fail "the daemon wrote a handshake and did not answer on port ${PORT}"

# It serves the interface. The window loads its assets from the app bundle rather than from here, so
# this is not what a user sees — it is the cheapest proof that the daemon in the bundle is the one
# built with the interface compiled in, which is a feature flag somebody can drop.
curl -fsS --max-time 10 "http://127.0.0.1:${PORT}/" | grep -q '<div id="root">' ||
  fail "the daemon is serving no interface — was the sidecar built without 'bundled'?"

# Still there. A daemon that starts, answers once and dies is the exact shape of a missing library
# opened lazily, and a check that stops at the first 200 would call that a pass.
sleep 5
kill -0 "${APP_PID}" 2>/dev/null || fail "the app exited within five seconds of starting"
curl -fsS --max-time 10 "http://127.0.0.1:${PORT}/health" > /dev/null ||
  fail "the daemon answered once and then stopped"

echo "smoke ok: the app started, its daemon answered on ${PORT}, and both were alive five seconds later"
