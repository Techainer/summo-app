#!/usr/bin/env bash
#
# Put the daemon where the Tauri shell expects to find it.
#
#   ./scripts/sidecar.sh            # debug, for `pnpm -C apps/desktop dev`
#   ./scripts/sidecar.sh --release  # for `pnpm -C apps/desktop build`
#
# Tauri ships helper executables as `externalBin`, and it looks for them under a name ending in the
# target triple — `summo-engine-x86_64-unknown-linux-gnu`, not `summo-engine`. That is how one
# bundle can carry a binary per architecture, and it is not something `cargo build` produces.
#
# Nothing did produce it. A clean checkout running `pnpm -C apps/desktop build` got
#
#   resource path `binaries/summo-engine-x86_64-unknown-linux-gnu` doesn't exist
#
# which names the file and not the fact that no documented command anywhere creates one. The
# desktop shell was unbuildable from a fresh clone, and stayed that way because the release
# workflow builds the CLI tarball instead and never touches it.
#
# `beforeBuildCommand` and `beforeDevCommand` in `tauri.conf.json` call this, so it is not a step
# anybody has to remember.

set -euo pipefail

cd "$(dirname "$0")/.."

PROFILE="debug"
CARGO_PROFILE=()
if [[ "${1:-}" == "--release" ]]; then
  PROFILE="release"
  CARGO_PROFILE=(--release)
fi

# `bundled` so the daemon carries the interface as well. The window is loaded from the app's own
# bundle rather than from the daemon, so this is not what puts a screen in front of the user — it is
# what lets `summo serve` from the same binary open one in a browser, and what the browser suites
# run against.
FEATURES="bundled,mcp,models,dub"

# Opus is linked *into* the daemon, not borrowed from the machine that built it — the same line
# `scripts/bundle.sh` has carried for a year, and the same reason. `audiopus_sys` links the system
# libopus when pkg-config finds one, which it does on every developer machine and every CI runner
# and does not on a stranger's laptop. Without this the installer carries a daemon that dies with
#
#   error while loading shared libraries: libopus.so.0
#
# on a machine that does not happen to have it. The tarball learned this the hard way; this script
# was written later and inherited none of it.
export OPUS_STATIC=1

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
OUT="apps/desktop/src-tauri/binaries"

# Models inside the installer: off by default, and not because they were a bad idea.
#
# They went in to route around a download that could not be relied on — `huggingface.co` and
# `github.com` are both regularly unreachable from Vietnamese consumer ISPs, so a first run could
# show a catalogue and install nothing from it. That is now fixed where it was broken: every weight
# file we are allowed to host has a third address on `summo.techainer.com`, which is Cloudflare and
# reaches everywhere the site does. The download works, so the installer does not have to carry
# 76 MB on the chance that it will not.
#
# Still here for the case it was actually built for: an offline install, a machine that will never
# see the network, an internal mirror. Set the variable and the models ride along again.
#
#     SUMMO_BUNDLE_MODELS=1 ./scripts/sidecar.sh
#
# `./scripts/…`, not `$(dirname "$0")/…`: this script has already `cd`-ed to the repository root by
# the time it gets here, so a path relative to where it was *invoked* points at nothing. Tauri runs
# it as `bash ../../scripts/sidecar.sh` from `apps/desktop/src-tauri`, which made this
#
#     bash: ../../scripts/bundle-models.sh: No such file or directory
#
# on every platform in the release job — and not on the machine it was written on, where it was run
# from the root.
if [[ "${SUMMO_BUNDLE_MODELS:-0}" == "1" ]]; then
  bash ./scripts/bundle-models.sh
else
  # A stale directory from an earlier build would be bundled by `tauri.conf.json` regardless of
  # this switch — the resource glob does not know why the files are there.
  rm -rf "${OUT}/models"
  echo "models: downloaded on first run (SUMMO_BUNDLE_MODELS=1 to ship them inside)"
fi

echo "building summo-engine (${PROFILE}, ${FEATURES})"
cargo build "${CARGO_PROFILE[@]}" --bin summo-engine --features "${FEATURES}"

mkdir -p "${OUT}"
cp "target/${PROFILE}/summo-engine" "${OUT}/summo-engine-${TARGET}"
echo "staged ${OUT}/summo-engine-${TARGET}"

# The libraries it loads at start, beside it.
#
# The daemon is linked against ONNX Runtime and sherpa-onnx — C++ libraries, not Rust crates — with
# a runpath of `$ORIGIN`. Staging the executable alone produced a desktop bundle that installed,
# opened, and killed its own engine the moment it started it:
#
#   error while loading shared libraries: libsherpa-onnx-c-api.so
#
# and nothing said so, because nothing had ever run the packaged app. `scripts/bundle.sh` had solved
# this for the CLI tarball years earlier; this script was written later and did not.
#
# They are listed as `bundle.resources` in `tauri.conf.json`, which puts them in `lib/` under the
# app's resource directory, and `apps/desktop/src-tauri/src/engine.rs` points the daemon's loader
# there — the resource directory is not the directory the sidecar itself ends up in on Linux or
# macOS.
#
# Asked of the linker rather than globbed: copying every `.so` in `target/` would sweep in build
# script leftovers, and this ends up inside every installer.
echo "staging the libraries it loads"

BIN="target/${PROFILE}/summo-engine"

# What the binary *says* it needs, before asking where any of it is.
#
# The earlier version of this asked `ldd`, which answers only when the loader can already find the
# library — and on CI it cannot: the prebuilt sherpa-onnx lives in `~/.cache/sherpa-rs`, the runpath
# says `$ORIGIN`, and `ldd` prints `=> not found`. That matched no pattern, staged nothing, and
# produced a bundle whose daemon could not start, with no error anywhere. Reading the declared
# dependencies instead means the list is right whether or not this machine can resolve them.
needs_of() {
  local file="$1"
  case "$(uname -s)" in
    Darwin)
      otool -L "${file}" | awk '/@rpath\// {sub(/.*@rpath\//, "", $1); print $1}'
      ;;
    MINGW* | MSYS* | CYGWIN*)
      # No dependency query worth trusting here; the DLLs land beside the binary and are the only
      # ones in that directory.
      find "target/${PROFILE}" -maxdepth 1 -name '*.dll' -printf '%f\n' 2>/dev/null || true
      ;;
    *)
      readelf -d "${file}" 2>/dev/null |
        awk '/NEEDED/ {gsub(/[][]/, "", $NF); print $NF}' |
        grep -Ev '^(libc|libm|libdl|libpthread|librt|libgcc_s|libstdc\+\+|ld-linux)' || true
      ;;
  esac
}

# Where that named library actually is. The loader first, since it is authoritative when it works,
# then the places a build script puts a prebuilt C++ library on each platform.
locate_lib() {
  local name="$1" found=""
  case "$(uname -s)" in
    Darwin) found="$(find "target/${PROFILE}" -maxdepth 2 -name "${name}" -type f 2>/dev/null | head -1)" ;;
    MINGW* | MSYS* | CYGWIN*) found="target/${PROFILE}/${name}" ;;
    *)
      found="$(ldd "${BIN}" | awk -v n="${name}" '$1 == n && $3 ~ /^\// {print $3}' | head -1)"
      ;;
  esac
  if [[ -z "${found}" || ! -f "${found}" ]]; then
    found="$(find "target/${PROFILE}" "${HOME}/.cache/sherpa-rs" "${HOME}/.cache/ort.pyke.io" \
      "${HOME}/Library/Caches/sherpa-rs" "${HOME}/Library/Caches/ort.pyke.io" \
      -name "${name}" -type f 2>/dev/null | head -1)"
  fi
  [[ -n "${found}" && -f "${found}" ]] && printf '%s' "${found}"
}

rm -rf "${OUT}/lib"
mkdir -p "${OUT}/lib"

# Never empty. `tauri-build` fails outright — not warns — on a resource pattern that matches
# nothing, and whether there is anything to stage depends on how this machine built sherpa-onnx: a
# static link leaves nothing behind, and a build that borrowed the prebuilt library leaves two
# files. A directory that exists and explains itself costs 200 bytes inside the installer and turns
# a broken build into a readable one.
cat > "${OUT}/lib/README" <<'NOTE'
The libraries the daemon loads at start — ONNX Runtime and sherpa-onnx — when this machine's build
borrowed them rather than linking them in. Staged by scripts/sidecar.sh, shipped as a bundle
resource, and put on the daemon's library path by apps/desktop/src-tauri/src/engine.rs.

Empty apart from this file means the build linked them statically. Both are fine.
NOTE

# Breadth-first, because a dependency has dependencies.
#
# `libonnxruntime.so` is not named by the daemon at all — it is named by `libsherpa-onnx-c-api.so`.
# A pass over the binary alone stages the one and leaves the other out, which is a bundle that gets
# exactly as far as loading sherpa and then stops.
staged=0
missing=""
seen=""
queue="$(needs_of "${BIN}")"
while [[ -n "${queue// /}" ]]; do
  name="$(printf '%s\n' ${queue} | head -1)"
  queue="$(printf '%s\n' ${queue} | tail -n +2)"
  case " ${seen} " in *" ${name} "*) continue ;; esac
  seen="${seen} ${name}"

  path="$(locate_lib "${name}")"
  if [[ -z "${path}" ]]; then
    missing="${missing} ${name}"
    continue
  fi
  # A library the operating system provides is the operating system's to provide. Copying
  # `/usr/lib/…` into the bundle ships one machine's glibc-linked build to every other one, and the
  # `.deb` would rather depend on the package than carry a stranger's copy of it.
  case "${path}" in
    /usr/* | /lib/* | /lib64/*)
      echo "leaving ${name} to the system (${path})"
      continue
      ;;
  esac
  cp "${path}" "${OUT}/lib/"
  echo "staged ${OUT}/lib/${name}"
  staged=$((staged + 1))
  queue="${queue} $(needs_of "${path}")"
done

# A hard failure, and this is the point of the rewrite. A library the binary declares and the bundle
# does not contain is an app that installs, opens, and cannot start its engine — discovered by
# whoever downloaded it. Better to fail here, where somebody is reading the output.
if [[ -n "${missing}" ]]; then
  echo "cannot find:${missing}" >&2
  echo "the daemon declares these and they are not in target/${PROFILE} or any known cache." >&2
  exit 1
fi
if [[ "${staged}" -eq 0 ]]; then
  echo "no libraries to stage — this build links them in"
fi

# Linux is told inside the binary, because the AppImage bundler reads the binary and nothing else.
#
# `linuxdeploy` walks every ELF in the AppDir and resolves what it declares. The staged daemon
# declares `libsherpa-onnx-c-api.so` with a runpath of `$ORIGIN`, and inside a bundle it sits in
# `usr/bin` while its libraries are in `usr/lib/Summo/lib` — so the bundler stopped with
#
#   ERROR: Could not find dependency: libsherpa-onnx-c-api.so
#
# reported by Tauri as nothing but "failed to run linuxdeploy". The `.deb` bundled fine either way,
# which is why the first release built one format and failed the other.
#
# One extra runpath entry answers it in both formats and at runtime: `usr/bin/../lib/Summo/lib` is
# `/usr/lib/Summo/lib` in the `.deb` and the same relative path inside the AppDir. The environment
# variable the shell sets stays as well — it costs nothing and it is what makes a build without
# `patchelf` still run.
if [[ "$(uname -s)" == "Linux" && "${staged}" -gt 0 ]]; then
  if command -v patchelf >/dev/null 2>&1; then
    patchelf --set-rpath '$ORIGIN:$ORIGIN/../lib/Summo/lib' "${OUT}/summo-engine-${TARGET}"
    echo "runpath set to \$ORIGIN:\$ORIGIN/../lib/Summo/lib"
  else
    # Not fatal: the `.deb` and a development run work without it. Only the AppImage needs it, and
    # only at bundle time.
    echo "patchelf not installed — the .deb will work and 'tauri build --bundles appimage' will not"
  fi
fi

# macOS finds them without being told, because it cannot be told.
#
# The shell puts the resource directory on the daemon's library path, which works on Linux and
# Windows and does not work on a notarised Mac: the hardened runtime strips every `DYLD_*` variable
# from a child process, precisely so that a library cannot be injected into a signed binary. The
# variable is not ignored — it is removed — and the daemon then dies looking for a dylib that is in
# the bundle.
#
# An `LC_RPATH` inside the binary survives that, because it is part of what was signed. In a macOS
# bundle the sidecar is `Contents/MacOS/summo-engine` and its resources are `Contents/Resources/`,
# so one relative entry covers every install location.
#
# Added to the staged copy, never to `target/`: a cargo artefact with an extra rpath would be reused
# by the next build of the CLI, which has no bundle around it.
if [[ "$(uname -s)" == "Darwin" && "${staged}" -gt 0 ]]; then
  install_name_tool -add_rpath "@executable_path/../Resources/lib" \
    "${OUT}/summo-engine-${TARGET}" 2>/dev/null &&
    echo "added @executable_path/../Resources/lib to the staged binary" ||
    echo "rpath already present, or install_name_tool unavailable"

  # And signed, because `install_name_tool` invalidates whatever signature the binary had, and
  # because what is under `Contents/Resources/` is not code as far as the bundler is concerned:
  # Tauri signs `Contents/MacOS` and leaves these two files exactly as they were copied.
  #
  # What they were copied with is a *linker* signature — the ad-hoc one `ld` puts on every arm64
  # build so the loader will map it at all — over a universal file whose Intel half is not signed
  # at all. That is enough to load and not enough to say who they belong to, which is the question
  # the hardened runtime asks. Signing them here answers it in the bundle rather than hoping.
  #
  # The same identity Tauri will use, so a Developer ID build signs these with the certificate and
  # an ordinary build signs them ad-hoc. `--force` because there is already a signature to replace,
  # and `--timestamp=none` because ad-hoc signing must not reach for Apple's timestamp server.
  identity="${APPLE_SIGNING_IDENTITY:--}"
  stamp=(--timestamp=none)
  [[ "${identity}" != "-" ]] && stamp=(--timestamp)
  for lib in "${OUT}/lib/"*.dylib; do
    [[ -e "${lib}" ]] || continue
    codesign --force --sign "${identity}" "${stamp[@]}" "${lib}" 2>&1 |
      sed "s|^|codesign: |" || {
      echo "could not sign ${lib}" >&2
      exit 1
    }
  done
fi
