#!/usr/bin/env bash
#
# Fetch the ONNX Runtime that Intel macOS builds load at runtime.
#
#   ./scripts/onnxruntime-intel-mac.sh <directory to put libonnxruntime.dylib in>
#
# Every other platform gets a runtime from `ort-sys`, which downloads one at build time from a table
# of prebuilts. `x86_64-apple-darwin` is not in that table, because Microsoft stopped publishing a
# build for it: 1.23.2 is the last ONNX Runtime release carrying `onnxruntime-osx-x86_64`, and
# everything after it ships `osx-arm64` alone.
#
# So the Intel build links nothing at compile time (`ort`'s `load-dynamic` feature) and opens this
# file at startup instead — `summo_core::onnx::locate_runtime` looks for it beside the executable.
# 1.23.2 answers `ORT_API_VERSION` 17, which is the version this workspace asks for, so nothing is
# missing at runtime; it is an older build of the same runtime, not a lesser one.
set -euo pipefail

DEST="${1:?usage: onnxruntime-intel-mac.sh <destination directory>}"
VERSION="1.23.2"
NAME="onnxruntime-osx-x86_64-${VERSION}"
URL="https://github.com/microsoft/onnxruntime/releases/download/v${VERSION}/${NAME}.tgz"

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

echo "fetching ${NAME}"
curl -fsSL --retry 3 --retry-delay 2 "${URL}" -o "${WORK}/ort.tgz"
tar -xzf "${WORK}/ort.tgz" -C "${WORK}"

# The tarball carries a versioned dylib and a symlink to it. Copy the real file under the plain
# name: a symlink into a directory that will be packaged separately is a dangling link on the other
# machine.
SOURCE="${WORK}/${NAME}/lib/libonnxruntime.${VERSION}.dylib"
[[ -f "${SOURCE}" ]] || {
  echo "the tarball did not contain ${SOURCE##*/}:" >&2
  ls -l "${WORK}/${NAME}/lib" >&2
  exit 1
}

mkdir -p "${DEST}"
cp -f "${SOURCE}" "${DEST}/libonnxruntime.dylib"
# `@rpath`-relative, so a binary beside it finds it wherever the app is installed. Without this the
# dylib carries the absolute path of this runner's temporary directory.
install_name_tool -id "@rpath/libonnxruntime.dylib" "${DEST}/libonnxruntime.dylib" 2> /dev/null || true
echo "installed $(ls -l "${DEST}/libonnxruntime.dylib" | awk '{print $5}') bytes at ${DEST}/libonnxruntime.dylib"
