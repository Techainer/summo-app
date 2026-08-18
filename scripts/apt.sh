#!/usr/bin/env bash
#
# Make `apt` survivable on a runner, and optionally install something.
#
#   ./scripts/apt.sh                    # only fix the configuration
#   ./scripts/apt.sh libasound2-dev     # fix it, then install
#
# Four Linux jobs in one evening died like this, after forty-four minutes:
#
#     Get:1 file:/etc/apt/apt-mirrors.txt Mirrorlist [144 B]
#     Ign:2 http://azure.archive.ubuntu.com/ubuntu noble InRelease
#     …
#     Error: The operation was canceled.
#
# `Ign` is apt reporting that a mirror did not answer and it will try the next one. With no
# `Acquire::*::Timeout` set it waits out the operating system's, which is minutes per index file,
# and the runner images route every Ubuntu source through one Azure mirror that fails as a unit.
# Nothing was wrong with the change being tested; what a developer sees is "the browser suites were
# cancelled".
#
# Two things are needed, and the first matters more than the second.
#
# **The configuration is global.** Half the apt calls in these workflows are not ours —
# `playwright install --with-deps` runs its own, and that is the one that burned forty-four minutes.
# So the timeouts and the mirror choice are written into `/etc/apt/apt.conf.d` and
# `/etc/apt/sources.list`, where every later apt inherits them, rather than passed as flags to the
# calls we happen to own.
#
# **The install comes before the update.** `apt-get update` is not what installs a package; it
# refreshes the index, and the runner image ships with one that is a few days old and perfectly able
# to resolve `libasound2-dev`. So the install is tried against what is already on the machine, and
# the archive is only asked when that fails.
set -euo pipefail

# Timeouts and retries, for every apt on this machine from here on — ours and everybody else's.
sudo tee /etc/apt/apt.conf.d/99-summo-timeouts > /dev/null <<'CONF'
Acquire::http::Timeout "15";
Acquire::https::Timeout "15";
Acquire::ftp::Timeout "15";
Acquire::Retries "2";
CONF

# Is the mirror the image points at actually answering? One request, five seconds, and the answer
# decides whether anything below has a chance.
#
# Checked rather than assumed: on a good day the Azure mirror is the fastest thing on the network,
# and rewriting the sources to the canonical archive would make every job slower for no reason.
MIRROR="$(sed -n '1s|/*$||p' /etc/apt/apt-mirrors.txt 2> /dev/null || true)"
if [[ -n "${MIRROR}" ]] && ! curl -fsS -m 5 -o /dev/null "${MIRROR}/dists/noble/InRelease"; then
  echo "apt: ${MIRROR} did not answer in five seconds — using the canonical archive" >&2
  # Moved aside rather than rewritten: whatever `sources.list` says still applies, and its default
  # is `archive.ubuntu.com`.
  sudo mv /etc/apt/apt-mirrors.txt /etc/apt/apt-mirrors.txt.slow 2> /dev/null || true
  sudo sed -i 's|file:/etc/apt/apt-mirrors.txt|http://archive.ubuntu.com/ubuntu|g' \
    /etc/apt/sources.list /etc/apt/sources.list.d/*.list \
    /etc/apt/sources.list.d/*.sources 2> /dev/null || true
fi

# Configuration only. Called this way before `playwright install --with-deps`, which installs its
# own list of packages and cannot be handed our flags.
[[ $# -gt 0 ]] || {
  echo "apt: configured"
  exit 0
}

install() { sudo timeout 900 apt-get install -y "$@"; }

# The index the image came with. Usually enough, and it costs nothing to find out.
if install "$@"; then
  echo "apt: installed from the index the image shipped with"
  exit 0
fi

echo "apt: that needed a fresher index" >&2
sudo timeout 240 apt-get update || true
install "$@"
