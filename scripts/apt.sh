#!/usr/bin/env bash
#
# `apt-get install`, for a machine whose package mirror is somebody else's problem.
#
#   ./scripts/apt.sh libasound2-dev
#
# Every Linux job here starts by installing two or three development packages, and three times in
# one evening a job sat inside `apt-get update` until its own timeout killed it:
#
#     Get:1 file:/etc/apt/apt-mirrors.txt Mirrorlist [144 B]
#     Ign:2 http://azure.archive.ubuntu.com/ubuntu noble InRelease
#     …
#     ##[error]The operation was canceled.
#
# `Ign` is apt saying a mirror did not answer and it will try the next one; with no timeout set it
# waits out the operating system's, which is minutes per index file. Nothing was wrong with the
# change being tested, and what a developer sees is "the desktop shell does not compile".
#
# The order here is the fix. `apt-get update` is not what installs anything — it refreshes the
# index, and the runner images ship with one that is a few days old and perfectly able to resolve
# `libasound2-dev`. So the install is tried first, against what is already on the machine, and the
# network is only involved when that fails. On a good day this touches the archive once instead of
# once per job; on a bad one it does not touch it at all.
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
  -o Acquire::Retries=2
  -o APT::Get::Assume-Yes=true
)

install() { sudo timeout 900 apt-get "${options[@]}" install -y "$@"; }
# `timeout` on top of apt's own: `Acquire::Retries` multiplies the wait rather than bounding it.
update() { sudo timeout 240 apt-get "${options[@]}" update; }

# The index the image came with. Usually enough, and it costs nothing to find out.
if install "$@"; then
  echo "apt: installed from the index the image shipped with"
  exit 0
fi

echo "apt: that needed a fresher index" >&2
if ! update; then
  echo "apt: the configured mirror did not answer — falling back to the canonical archive" >&2
  # The runner images resolve every source through this list. Moved aside rather than rewritten:
  # whatever `sources.list` says still applies, and its default is `archive.ubuntu.com`.
  sudo mv /etc/apt/apt-mirrors.txt /etc/apt/apt-mirrors.txt.slow 2>/dev/null || true
  sudo sed -i 's|file:/etc/apt/apt-mirrors.txt|http://archive.ubuntu.com/ubuntu|g' \
    /etc/apt/sources.list /etc/apt/sources.list.d/*.list \
    /etc/apt/sources.list.d/*.sources 2>/dev/null || true
  # Ubuntu's own archive only. The image also carries Microsoft's, Google's and a handful of PPAs,
  # and none of them has ever held a package this repository asks for — they are three more chances
  # to hang.
  update || sudo timeout 240 apt-get "${options[@]}" \
    -o Dir::Etc::sourceparts=/dev/null update
fi

install "$@"
