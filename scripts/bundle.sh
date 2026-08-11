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

FEATURES="bundled,mcp,models"
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
cp target/release/summo "${OUT}/"

# Only what the binary actually asks for. Copying every `.so` in `target/release` would sweep in
# build-script leftovers and make the tarball larger than the thing it contains.
if [[ "${FEATURES}" == *models* ]]; then
  for lib in $(ldd target/release/summo | awk '/=> \/.*target\/release/ {print $3}'); do
    cp "${lib}" "${OUT}/"
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

echo "==> archiving"
mkdir -p dist
tar -C dist -czf "dist/${NAME}.tar.gz" "${NAME}"

# A checksum beside the archive, so somebody who did not build it can tell whether they got what we
# published.
( cd dist && sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256" )

echo
du -sh "dist/${NAME}.tar.gz" | awk '{print "   " $1 "\t" $2}'
cat "dist/${NAME}.tar.gz.sha256"
