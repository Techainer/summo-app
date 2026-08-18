#!/usr/bin/env bash
#
# `apt-get install`, for a machine whose package mirror is somebody else's problem.
#
#   ./scripts/apt.sh libasound2-dev
#
# Every Linux job here starts by installing two or three development packages, and twice in one
# evening the workflow spent twenty-seven minutes inside `apt-get update` before the job's own
# timeout killed it:
#
#     Get:1 file:/etc/apt/apt-mirrors.txt Mirrorlist [144 B]
#     Ign:2 http://azure.archive.ubuntu.com/ubuntu noble InRelease
#     …
#     ##[error]The operation was canceled.
#
# `Ign` is apt saying it could not reach a mirror and will try the next one; with no timeout set it
# waits out the operating system's, which is minutes per index. Nothing was wrong with the change
# being tested, and the report a developer gets is "the desktop shell does not compile".
#
# So: a short network timeout, apt's own retries, and one attempt at the whole thing again with the
# mirrorlist put aside — the runner images point at an Azure mirror that fails as a unit, and the
# canonical archive is reachable when it is not.
set -euo pipefail

[[ $# -gt 0 ]] || {
  echo "usage: apt.sh <package>…" >&2
  exit 2
}

# Seconds apt waits on a connection before giving up on that mirror, and how many times it retries
# each index. Both are absent by default, which is what makes a bad mirror a hung job.
options=(
  -o Acquire::http::Timeout=15
  -o Acquire::https::Timeout=15
  -o Acquire::ftp::Timeout=15
  -o Acquire::Retries=3
)

update() {
  # `timeout` on top of apt's own: `Acquire::Retries` multiplies the wait rather than bounding it.
  sudo timeout 180 apt-get "${options[@]}" update
}

if ! update; then
  echo "apt: the configured mirror did not answer — falling back to the canonical archive" >&2
  # The runner images resolve every source through this list. Moved aside rather than rewritten:
  # whatever is in `sources.list` still applies, and the default there is `archive.ubuntu.com`.
  sudo mv /etc/apt/apt-mirrors.txt /etc/apt/apt-mirrors.txt.slow 2>/dev/null || true
  sudo sed -i 's|file:/etc/apt/apt-mirrors.txt|http://archive.ubuntu.com/ubuntu|g' \
    /etc/apt/sources.list /etc/apt/sources.list.d/*.list \
    /etc/apt/sources.list.d/*.sources 2>/dev/null || true
  update
fi

sudo timeout 600 apt-get "${options[@]}" install -y "$@"
