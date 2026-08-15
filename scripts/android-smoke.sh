#!/usr/bin/env bash
#
# Install the Android app on a running device and prove it reached its own engine.
#
#   ./scripts/android-smoke.sh apps/mobile/.../app-universal-release.apk
#
# The engine on mobile is linked into the app process and the webview talks to it over loopback —
# `http://127.0.0.1:<port>`, same routes and same token as the desktop sidecar. Everything about
# that arrangement can be right and the app still show nothing, which is what happened: a release
# build sets `usesCleartextTraffic="false"`, Android refused every request to the app's own socket,
# and the first screen said "Failed to fetch" over a daemon that was listening perfectly well. The
# debug build sets the flag to "true", so the only configuration that worked was the one nobody
# ships.
#
# What is asserted is therefore not "the app opened" but "the webview got an answer from the
# engine": the setup screen prints the *device's own* core count and memory, and that sentence
# cannot be drawn without a successful call over loopback. It is read out of the accessibility
# tree, which is where a WebView publishes its text — no screenshots, no OCR.

set -euo pipefail

APK="${1:?usage: android-smoke.sh <path to .apk>}"
PKG="${2:-app.summo.mobile}"
ADB="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/android-sdk}}/platform-tools/adb"
[[ -x "${ADB}" ]] || ADB="adb"

"${ADB}" wait-for-device
echo "device: $("${ADB}" shell getprop ro.build.version.release | tr -d '\r') on $("${ADB}" shell getprop ro.product.cpu.abi | tr -d '\r')"

"${ADB}" uninstall "${PKG}" > /dev/null 2>&1 || true
"${ADB}" install -r "${APK}"
"${ADB}" logcat -c

# `monkey` rather than `am start`: it finds the launcher activity itself, so the class name is not
# a thing this script has to know and keep in step with the Tauri template.
"${ADB}" shell monkey -p "${PKG}" -c android.intent.category.LAUNCHER 1 > /dev/null

# The webview has to boot, ask for the handshake, and get an answer. Sixty seconds because an
# emulator on a shared runner is slower than any phone.
found=""
for _ in $(seq 1 30); do
  sleep 2
  # Deleted before every dump, and `uiautomator` is not trusted to have written one.
  #
  # It exits 0 whether or not it produced anything — "ERROR: could not get idle state" is a normal
  # thing for it to print while an app is still animating — so a run that dumped nothing left the
  # *previous* run's file on the device. This script passed twice against a build that could not
  # reach its engine at all, reading a screen from ten minutes earlier.
  "${ADB}" shell rm -f /sdcard/summo-ui.xml > /dev/null 2>&1 || true
  "${ADB}" shell uiautomator dump /sdcard/summo-ui.xml > /dev/null 2>&1 || continue
  screen="$("${ADB}" shell cat /sdcard/summo-ui.xml 2>/dev/null || true)"
  [[ "${screen}" == *"<hierarchy"* ]] || continue

  # The failure this exists for, reported as itself rather than as a timeout.
  if [[ "${screen}" == *"Failed to fetch"* || "${screen}" == *"Load failed"* ]]; then
    echo "::error::the app could not reach its own engine — see network_security_config.xml"
    break
  fi
  # "This machine: 4 cores, 4 GB RAM." — the daemon's answer, in the app's words.
  if [[ "${screen}" == *"cores,"* && "${screen}" == *"RAM"* ]]; then
    found="$(printf '%s' "${screen}" | grep -o 'text="This machine[^"]*"' | head -1)"
    break
  fi
done

if [[ -z "${found}" ]]; then
  echo "--- what was on screen ---"
  "${ADB}" shell cat /sdcard/summo-ui.xml 2>/dev/null | grep -o 'text="[^"]\+"' | head -30 || true
  echo "--- what the app said ---"
  "${ADB}" logcat -d | grep -viE "s_gl|glBind|EGL_emulation|OpenGLRenderer" | tail -40
  echo "the app did not reach its engine within 60s" >&2
  exit 1
fi

echo "the webview reached the engine: ${found}"

# Still there. An app that draws one screen and dies is not one that works, and the engine is in
# the same process — a panic in it takes the window with it.
sleep 5
"${ADB}" shell pidof "${PKG}" > /dev/null || {
  echo "--- what the app said ---"
  "${ADB}" logcat -d | grep -viE "s_gl|glBind|EGL_emulation" | tail -40
  echo "the app died within five seconds of drawing its first screen" >&2
  exit 1
}

echo "android smoke ok"
