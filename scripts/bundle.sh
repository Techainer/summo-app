#!/usr/bin/env bash
#
# Build a release bundle: everything a person needs to run Summo, and nothing else.
#
#   ./scripts/bundle.sh              # with speech recognition
#   ./scripts/bundle.sh --no-models  # smaller, browses a vault, cannot transcribe
#
# The output is a tarball that unpacks into a directory you can run from anywhere.
#
# ## Why a directory and not one file
#
# `summo serve` without recognition genuinely is one file — the interface is compiled in. With
# recognition it is not, and pretending otherwise would be a lie a user discovers by moving the
# binary somewhere else and watching it fail to start.
#
# ONNX Runtime and sherpa-onnx are C++ libraries loaded at process start. Statically linking them is
# possible and unpleasant: it means building ONNX Runtime from source per platform, and it doubles
# the artefact for a link nobody benefits from. So they ship beside the binary, which already has
# `RUNPATH=$ORIGIN` and finds them there — the same shape `ollama` ships, for the same reason.
#
# ## What it does not do
#
# It does not download models. That is `summo serve`'s first-run screen, which ranks what fits this
# machine and says why. Baking a model in would make the download a gigabyte and choose for the
# user, which is exactly the choice the setup screen exists to hand back to them.

set -euo pipefail

cd "$(dirname "$0")/.."

# ---- the three things that differ per platform -------------------------------------------------
#
# This script used to be Linux-only while being run on three platforms. `ldd` does not exist on
# macOS and `sha256sum` does not exist on either macOS or Windows, so the release job for a Mac
# built the binary, failed at the checksum and published nothing — the kind of break that is only
# noticed on the day of a release, which is the day it costs the most.

case "$(uname -s)" in
  Darwin) PLATFORM=macos ;;
  MINGW* | MSYS* | CYGWIN*) PLATFORM=windows ;;
  *) PLATFORM=linux ;;
esac

# Windows names its executables. Everything else does not.
EXE=""
[[ "${PLATFORM}" == "windows" ]] && EXE=".exe"

# `sha256sum` on Linux and in Git Bash, `shasum -a 256` on macOS. Same digest, same output shape.
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$@"
  else
    shasum -a 256 "$@"
  fi
}

# The libraries the binary loads at start: ONNX Runtime and sherpa-onnx. Found by asking the linker
# what it wants rather than by copying every file in `target/release`, which would sweep in build
# script leftovers and make the archive larger than the thing inside it.
collect_libs() {
  case "${PLATFORM}" in
    linux)
      ldd "target/release/summo${EXE}" | awk '/=> \/.*target\/release/ {print $3}'
      ;;
    macos)
      # `@rpath/libfoo.dylib` — the loader resolves it against `$ORIGIN`, so the name is enough to
      # find the copy in `target/release` that it was linked against.
      otool -L "target/release/summo${EXE}" |
        awk '/@rpath\// {sub(/@rpath\//, "", $1); print "target/release/" $1}'
      ;;
    windows)
      # No linker query on Windows that is worth trusting here: the DLLs land beside the binary in
      # `target/release` and are the only DLLs there.
      find target/release -maxdepth 1 -name "*.dll" 2>/dev/null || true
      ;;
  esac
}

# Opus is linked *into* the binary, not borrowed from the machine that built it.
#
# `audiopus_sys` links the system libopus when pkg-config finds one — which it does on a CI runner
# and on most developer machines, and does not on a user's laptop. The first release built this way
# produced a tarball that died on launch with
#
#   error while loading shared libraries: libopus.so.0: cannot open shared object file
#
# on any machine without libopus installed. Nobody saw it locally for the obvious reason: everybody
# building it had libopus. `check_no_stray_deps` below is the guard that turns that class of
# mistake into a failed build rather than a failed download.
export OPUS_STATIC=1

# `mt-onnx` and not `local-mt`: the ONNX translator is pure Rust plus a prebuilt runtime, so it
# builds on every platform we release for, while llama.cpp needs a C++ toolchain and CMake on each
# of them. Without it the packaged app answers "this build cannot run a translation model
# in-process" — which is what v0.1.0 shipped, an offline-first product that could not translate
# offline unless you pointed it at somebody's server. The GGUF models it would add are not
# redistributable anyway.
FEATURES="bundled,mcp,models,dub,mt-onnx"
SUFFIX=""
if [[ "${1:-}" == "--no-models" ]]; then
  FEATURES="bundled,mcp"
  SUFFIX="-nomodels"
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
NAME="summo-${VERSION}-${TARGET}${SUFFIX}"
OUT="dist/${NAME}"

echo "==> interface"
# Compiled into the binary, so it has to exist first. A stale `dist/` here would ship yesterday's
# interface inside today's binary, which is the kind of mismatch nobody thinks to check.
pnpm install --frozen-lockfile >/dev/null
pnpm --filter @summo/web build >/dev/null

echo "==> binary (${FEATURES})"
cargo build --release -p summo-cli --features "${FEATURES}"

echo "==> collecting"
rm -rf "${OUT}"
mkdir -p "${OUT}"
cp "target/release/summo${EXE}" "${OUT}/"

# Only what the binary actually asks for. Copying every `.so` in `target/release` would sweep in
# build-script leftovers and make the tarball larger than the thing it contains.
if [[ "${FEATURES}" == *models* ]]; then
  for lib in $(collect_libs); do
    [[ -f "${lib}" ]] && cp "${lib}" "${OUT}/"
  done
fi

cp README.md LICENSE NOTICE "${OUT}/" 2>/dev/null || true

cat > "${OUT}/RUN.md" <<'EOF'
# Summo

    ./summo serve

That prints an address and opens it. Everything runs on this machine: speech recognition and
speaker attribution never leave it, and your notes are Markdown files in `~/.summo/vault` that you
can open in Obsidian, grep, back up or delete.

The first run asks one question — which speech model to download. Summo ranks what is available
against this machine and says why, so you can disagree with it. Nothing is recorded until you press
record.

    ./summo serve --port 8710     a fixed port
    ./summo serve --no-open       a server, when there is no browser here
    ./summo import recording.mp4  a meeting you already have
    ./summo mcp                   the vault as tools, for Claude Code or Cursor

Built by Viet Nguyen — CEO, Techainer (https://techainer.com/). AGPL-3.0.
EOF

# What the operating system does to a downloaded binary before it will run one.
#
# This is the first thing a new user meets and it looks like the app is broken, so it belongs in the
# file that ships beside the app rather than in a page they would have to already be reading. The
# macOS text is checked against what the binary actually carries: an ad-hoc signature the linker
# adds — `flags 0x20002`, no CMS blob — which is enough to execute but is not a Developer ID and is
# not notarised, so Gatekeeper refuses a *quarantined* copy.
case "${PLATFORM}" in
  macos)
    cat >> "${OUT}/RUN.md" <<'EOF'

---

## First launch on macOS

macOS refuses anything downloaded from a browser that Apple has not notarised, and says the
developer "cannot be verified" rather than what it is really objecting to. This build is ad-hoc
signed, which is not the same as notarised — we have no Apple Developer certificate yet.

To run it, strip the quarantine flag from the folder, once:

    xattr -dr com.apple.quarantine .
    ./summo serve

`-r`, because the two `.dylib` files beside the binary are quarantined too, and the app will fail
when recognition starts rather than when it launches.

The microphone prompt goes to the program that *started* Summo — the terminal — so if you never see
it, look in System Settings → Privacy & Security → Microphone for your terminal app.
EOF
    ;;
  windows)
    cat >> "${OUT}/RUN.md" <<'EOF'

---

## First launch on Windows

SmartScreen shows "Windows protected your PC" for any executable without a code-signing
certificate, and hides the button that runs it: click **More info**, then **Run anyway**.

Keep the `.dll` files in the same folder as `summo.exe`. Copying the executable somewhere on its own
gives a "code execution cannot proceed" dialog naming a DLL rather than anything about Summo.
EOF
    ;;
esac

# Only the recognition build has libraries beside the binary, and only it should say so. A note
# about `.so` files in a bundle that has none is the sort of thing that makes a reader distrust the
# rest of the page.
if [[ "${FEATURES}" == *models* ]]; then
  cat >> "${OUT}/RUN.md" <<'EOF'

---

The `.so` files beside the binary are ONNX Runtime and sherpa-onnx, loaded when recognition starts.
Keep them together: moving `summo` on its own will fail to launch.
EOF
else
  cat >> "${OUT}/RUN.md" <<'EOF'

---

This build has no speech recognition compiled in — it browses a vault, imports, summarises and
answers questions, but cannot transcribe. The full build is the one without `-nomodels` in its name.
EOF
fi

# ---- the guard ---------------------------------------------------------------------------------
#
# Whatever the binary still needs from the machine it lands on, and whether that is reasonable.
#
# Reasonable is: the C runtime, the C++ runtime, the maths and threading libraries every Linux
# binary links, and the two `.so` files shipped beside it. Anything else is a library that happened
# to be installed on the build machine, and a user without it gets a binary that will not start —
# which is exactly what shipped the first time.
check_no_stray_deps() {
  [[ "${PLATFORM}" == "linux" ]] || return 0
  local allowed="libc|libm|libdl|libpthread|librt|libgcc_s|libstdc\+\+|ld-linux|linux-vdso|libonnxruntime|libsherpa-onnx"
  local stray
  stray="$(ldd "${OUT}/summo${EXE}" | awk '{print $1}' | grep -Ev "${allowed}" | grep -E '\.so' || true)"
  if [[ -n "${stray}" ]]; then
    echo "!!! the binary depends on libraries this bundle does not ship:" >&2
    echo "${stray}" | sed 's/^/    /' >&2
    echo "    a user without them gets \"cannot open shared object file\" at launch." >&2
    exit 1
  fi
  echo "==> dependencies: only the C runtime and the libraries beside the binary"
}

check_no_stray_deps

echo "==> archiving"
mkdir -p dist

# A zip on Windows and a tarball everywhere else. Not taste: Windows unpacks a zip by double-click
# and needs a tool for a `.tar.gz`, and the first thing a new user does with a download is
# double-click it.
if [[ "${PLATFORM}" == "windows" ]]; then
  ARCHIVE="${NAME}.zip"
  ( cd dist && powershell -NoProfile -Command "Compress-Archive -Path '${NAME}' -DestinationPath '${ARCHIVE}' -Force" )
else
  ARCHIVE="${NAME}.tar.gz"
  tar -C dist -czf "dist/${ARCHIVE}" "${NAME}"
fi

# A checksum beside the archive, so somebody who did not build it can tell whether they got what we
# published.
( cd dist && sha256 "${ARCHIVE}" > "${ARCHIVE}.sha256" )

echo
du -sh "dist/${ARCHIVE}" | awk '{print "   " $1 "\t" $2}'
cat "dist/${ARCHIVE}.sha256"
