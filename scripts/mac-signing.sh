#!/usr/bin/env bash
#
# Whether this bundle can load its own libraries on somebody else's Mac.
#
#   ./scripts/mac-signing.sh apps/desktop/src-tauri/target/release/bundle/macos/Summo.app
#
# The smoke test starts the app and is not enough, because the machine that builds a bundle is the
# one machine that runs it leniently. `v0.2.6` started on the runner that built it, was published,
# and died on the first Mac it reached:
#
#   dyld: Library not loaded: @rpath/libonnxruntime.1.17.1.dylib
#     Reason: … not valid for use in process: mapping process and mapped file (non-platform)
#     have different Team IDs
#
# Nothing was wrong with the libraries. The bundle is signed with the hardened runtime, which turns
# on *library validation*: a process may load only code signed by its own team, or by Apple. Our
# signature is ad-hoc and has no team, so the rule permitted nothing but macOS itself — and the two
# libraries the daemon cannot start without sit in `Contents/Resources/lib`.
#
# That is a property of the signature rather than of the run, so this reads it instead of hoping to
# provoke it. Deterministic, a few seconds, and it would have failed on `v0.2.0` — the first bundle
# that ever shipped — while every behavioural check we had said the app was fine.
#
# Written for the bash macOS ships, which is 3.2: no `mapfile`, no associative arrays.
set -euo pipefail

APP="${1:?usage: mac-signing.sh <path to Summo.app>}"
[[ -d "${APP}" ]] || {
  echo "not a bundle: ${APP}" >&2
  exit 2
}

problems=0
note() {
  echo "mac-signing: $1" >&2
  problems=$((problems + 1))
}
short() { echo "${1#"${APP}"/}"; }

# The executables macOS will actually start: the window, and the daemon it spawns.
binaries=()
while IFS= read -r file; do binaries+=("${file}"); done < <(
  find "${APP}/Contents/MacOS" -maxdepth 1 -type f -perm -u+x
)
[[ "${#binaries[@]}" -gt 0 ]] || note "nothing executable in Contents/MacOS"

# The libraries that ride along. `Resources/lib` is where `sidecar.sh` stages them and where
# `engine.rs` points the daemon; `Frameworks` is where a Tauri bundle would put a framework.
libraries=()
while IFS= read -r file; do libraries+=("${file}"); done < <(
  find "${APP}/Contents/Resources/lib" "${APP}/Contents/Frameworks" \
    -type f -name '*.dylib' 2> /dev/null || true
)

echo "${#binaries[@]} executable(s), ${#libraries[@]} bundled library file(s)"

team_of() {
  local team
  team="$(codesign -dv --verbose=4 "$1" 2>&1 | sed -n 's/^TeamIdentifier=//p' | head -1)"
  # macOS says "not set" rather than saying nothing, and the two must not read differently here.
  [[ "${team}" == "not set" ]] && team=""
  printf '%s' "${team}"
}
flags_of() {
  codesign -dv --verbose=4 "$1" 2>&1 | sed -n 's/.*flags=\([^ ]*\).*/\1/p' | head -1
}
entitlements_of() {
  codesign -d --entitlements - "$1" 2>/dev/null || true
}

# Signed at all. An unsigned arm64 binary is not "unverified", it is refused — which is the failure
# before this one, reported by a user as "Summo is damaged".
for file in ${binaries[@]+"${binaries[@]}"} ${libraries[@]+"${libraries[@]}"}; do
  codesign --verify --strict "${file}" 2> /dev/null ||
    note "$(short "${file}") carries no valid signature — macOS will refuse to map it"
done

for binary in ${binaries[@]+"${binaries[@]}"}; do
  name="$(short "${binary}")"
  team="$(team_of "${binary}")"
  flags="$(flags_of "${binary}")"
  echo "  ${name}: flags=${flags:-?} team=${team:-none}"

  case "${flags}" in *runtime*) ;; *) continue ;; esac

  # The microphone. Under the hardened runtime an app not entitled to it records silence and says
  # nothing about why.
  entitlements="$(entitlements_of "${binary}")"
  grep -q "com.apple.security.device.audio-input" <<< "${entitlements}" ||
    note "${name} is hardened and not entitled to the microphone"

  # Library validation, which is the one that shipped broken. Either the bundle carries no
  # libraries of its own, or the process is entitled to load them, or they belong to its team —
  # and an ad-hoc signature has no team, so for an unsigned build the entitlement is the only door.
  [[ "${#libraries[@]}" -gt 0 ]] || continue
  grep -q "com.apple.security.cs.disable-library-validation" <<< "${entitlements}" && continue

  for lib in ${libraries[@]+"${libraries[@]}"}; do
    lib_team="$(team_of "${lib}")"
    if [[ -z "${team}" || "${lib_team}" != "${team}" ]]; then
      note "${name} runs under the hardened runtime with team '${team:-none}' and $(short "${lib}")
    has team '${lib_team:-none}'. Library validation will refuse it, and the app will die at
    startup on every machine except the one that built it. Either add
    com.apple.security.cs.disable-library-validation to Entitlements.plist, or sign both with the
    same certificate."
      break
    fi
  done
done

if [[ "${problems}" -gt 0 ]]; then
  echo "mac-signing: ${problems} problem(s)" >&2
  exit 1
fi
echo "mac-signing ok: every binary is signed, and the daemon may load the libraries beside it"
