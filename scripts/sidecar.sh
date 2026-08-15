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

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
OUT="apps/desktop/src-tauri/binaries"

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
case "$(uname -s)" in
  Darwin)
    LIBS="$(otool -L "target/${PROFILE}/summo-engine" |
      awk '/@rpath\// {sub(/@rpath\//, "", $1); print "target/'"${PROFILE}"'/" $1}')"
    ;;
  MINGW* | MSYS* | CYGWIN*)
    LIBS="$(find "target/${PROFILE}" -maxdepth 1 -name '*.dll')"
    ;;
  *)
    LIBS="$(ldd "target/${PROFILE}/summo-engine" | awk '/=> \/.*target\// {print $3}')"
    ;;
esac

# A directory of their own, and one that is never empty on any platform this builds for. The three
# globs this replaced — `*.so`, `*.dylib`, `*.dll` — were a build failure rather than a warning:
# `tauri-build` refuses a resource pattern that matches nothing, so two of the three broke the build
# on every platform.
rm -rf "${OUT}/lib"
mkdir -p "${OUT}/lib"
for lib in ${LIBS}; do
  [[ -f "${lib}" ]] || continue
  cp "${lib}" "${OUT}/lib/"
  echo "staged ${OUT}/lib/$(basename "${lib}")"
done
