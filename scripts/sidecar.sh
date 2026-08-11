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

# The interface has to be inside the daemon: the shell points a webview at it rather than serving
# the files itself, so a daemon without `bundled` gives a window with nothing in it.
FEATURES="bundled,mcp,models,dub"

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
OUT="apps/desktop/src-tauri/binaries"

echo "building summo-engine (${PROFILE}, ${FEATURES})"
cargo build "${CARGO_PROFILE[@]}" --bin summo-engine --features "${FEATURES}"

mkdir -p "${OUT}"
cp "target/${PROFILE}/summo-engine" "${OUT}/summo-engine-${TARGET}"
echo "staged ${OUT}/summo-engine-${TARGET}"
