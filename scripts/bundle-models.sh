#!/usr/bin/env bash
#
# Put the models the app needs inside the app.
#
#   ./scripts/bundle-models.sh
#
# A person downloads an installer and the first thing it does is ask for the network: measure the
# machine, list the models, wait for seventy megabytes. When the list cannot be fetched — which is
# an ordinary evening on a Vietnamese consumer ISP, where `raw.githubusercontent.com` is blocked —
# the setup screen says "Could not reach the model list" and there is nothing else to press.
#
# So the recogniser and the voice detector ride along. `summo-models`'s `seed` module copies them
# into the vault the first time the daemon starts, checking each digest on the way in, and after
# that they are ordinary installed models: removable, counted in the disk figure, indistinguishable
# from a model somebody chose.
#
# Roughly 76 MB, which takes the macOS installer from 40 MB to about 115. That is the trade, and it
# is the right way round: bandwidth once, at a moment when somebody has already decided to install
# something, rather than a wait and a possible dead end at the moment they first open it.
#
# Written into the layout the store already uses, so seeding is a copy and a checksum rather than a
# second install path:
#
#     binaries/models/manifests/<id>.json
#     binaries/models/blobs/sha256/<ab>/<digest>
set -euo pipefail

cd "$(dirname "$0")/.."

OUT="apps/desktop/src-tauri/binaries/models"
# The same chain the app uses, for the same reason: the first two are blocked from some networks
# and this script runs on a developer's machine as often as on a runner.
SOURCES=(
  "https://raw.githubusercontent.com/Techainer/summo-registry/main"
  "https://cdn.jsdelivr.net/gh/Techainer/summo-registry@main"
  "https://summo.techainer.com/registry"
)

# What ships. The recogniser the setup screen recommends on almost every machine, and the voice
# detector without which nothing is ever committed to a transcript — the pair is the smallest thing
# that makes recording work at all.
MODELS=("gipformer-65m" "silero-vad-v5")

mkdir -p "${OUT}/manifests" "${OUT}/blobs/sha256"

fetch() {
  local path="$1" dest="$2"
  for base in "${SOURCES[@]}"; do
    if curl -fsSL --max-time 600 --retry 2 "${base}/${path}" -o "${dest}"; then
      return 0
    fi
  done
  return 1
}

digest_of() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

for id in "${MODELS[@]}"; do
  manifest="${OUT}/manifests/${id}.json"
  echo "manifest ${id}"
  fetch "models/${id}.json" "${manifest}" || {
    echo "cannot fetch the manifest for ${id} from any source" >&2
    exit 1
  }

  # `python3` rather than `jq`: every platform that builds this already has it, and Windows runners
  # do not ship jq. The fields are the manifest's own — name, url, sha256 — and the mirrors are
  # tried in order, because HuggingFace is unreachable from some of the same networks GitHub is.
  python3 - "${manifest}" <<'PY' | while IFS=$'\t' read -r digest url mirrors; do
import json, sys
manifest = json.load(open(sys.argv[1], encoding="utf-8"))
for entry in manifest.get("files", []):
    mirrors = ",".join(entry.get("mirror", []))
    print(f"{entry['sha256']}\t{entry['url']}\t{mirrors}")
PY
    shard="${OUT}/blobs/sha256/${digest:0:2}"
    blob="${shard}/${digest}"
    mkdir -p "${shard}"

    if [[ -f "${blob}" && "$(digest_of "${blob}")" == "${digest}" ]]; then
      echo "  have ${digest:0:12}…"
      continue
    fi

    got=""
    IFS=',' read -ra alternates <<< "${mirrors}"
    for candidate in "${url}" "${alternates[@]}"; do
      [[ -n "${candidate}" ]] || continue
      echo "  fetching ${digest:0:12}… from ${candidate%%/*}//${candidate#*//}" | cut -c1-100
      if curl -fsSL --max-time 900 --retry 2 "${candidate}" -o "${blob}.part"; then
        got="${candidate}"
        break
      fi
    done
    [[ -n "${got}" ]] || {
      echo "  could not fetch ${digest} from any address" >&2
      exit 1
    }

    # Checked here, and checked again by `seed` when it copies into the vault. Twice is not
    # redundant: this one catches a bad download, that one catches a rewritten installer.
    actual="$(digest_of "${blob}.part")"
    if [[ "${actual}" != "${digest}" ]]; then
      echo "  ${got} does not match its manifest (${actual})" >&2
      rm -f "${blob}.part"
      exit 1
    fi
    mv "${blob}.part" "${blob}"
  done
done

size="$(du -sh "${OUT}" | cut -f1)"
echo "bundled ${#MODELS[@]} models, ${size} in ${OUT}"
