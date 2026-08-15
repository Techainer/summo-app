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
    # Anything the loader resolves outside a system directory. Matching on `target/` alone was
    # wrong: whether sherpa-onnx is linked statically or borrowed from `~/.cache/sherpa-rs` depends
    # on how the machine built it, and on CI it is not under `target/` at all.
    LIBS="$(ldd "target/${PROFILE}/summo-engine" |
      awk '/=> \// {print $3}' | grep -Ev '^/(usr|lib|lib64)/' || true)"
    ;;
esac

# A directory of their own, and one that is never empty on any platform this builds for. The three
# globs this replaced — `*.so`, `*.dylib`, `*.dll` — were a build failure rather than a warning:
# `tauri-build` refuses a resource pattern that matches nothing, so two of the three broke the build
# on every platform.
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

staged=0
for lib in ${LIBS}; do
  [[ -f "${lib}" ]] || continue
  cp "${lib}" "${OUT}/lib/"
  echo "staged ${OUT}/lib/$(basename "${lib}")"
  staged=$((staged + 1))
done
if [[ "${staged}" -eq 0 ]]; then
  echo "no libraries to stage — this build links them in"
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
fi
