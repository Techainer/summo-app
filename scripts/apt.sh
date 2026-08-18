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
# `Acquire::*::Timeout` set it waits out the operating system's, which is minutes per index file.
# Nothing was wrong with the change being tested; what a developer sees is "the browser suites were
# cancelled".
#
# Two things fix that, and neither is switching mirrors.
#
# **The configuration is global.** Half the apt calls in these workflows are not ours —
# `playwright install --with-deps` runs its own, and that is the one that burned forty-four minutes.
# So the timeouts go into `/etc/apt/apt.conf.d`, where every later apt inherits them, rather than
# being passed as flags to the calls we happen to own.
#
# **The install comes before the update.** `apt-get update` is not what installs a package; it
# refreshes the index, and the runner image ships with one that is a few days old and perfectly able
# to resolve `libasound2-dev`. So the install is tried against what is already on the machine, and
# the archive is only asked when that fails.
#
# What this deliberately does *not* do is rewrite the sources to a different mirror. The version
# that did took seven jobs down at once, and the reason is worth keeping: apt's cached index is
# keyed by the source URL, so changing the URL throws away the index the image shipped with — which
# is the one thing here that reliably works. It made the fast path impossible in order to make the
# slow path slightly less slow, and the slow path was down anyway.
set -euo pipefail

# Timeouts and retries, for every apt on this machine from here on — ours and everybody else's.
sudo tee /etc/apt/apt.conf.d/99-summo-timeouts > /dev/null <<'CONF'
Acquire::http::Timeout "15";
Acquire::https::Timeout "15";
Acquire::ftp::Timeout "15";
Acquire::Retries "2";
CONF

# Configuration only. Called this way before `playwright install --with-deps`, which installs its
# own list of packages and cannot be handed our flags.
[[ $# -gt 0 ]] || {
  echo "apt: timeouts configured"
  exit 0
}

install() { sudo timeout 900 apt-get install -y "$@"; }

# The index the image came with. Usually enough, and it costs nothing to find out.
if install "$@"; then
  echo "apt: installed from the index the image shipped with"
  exit 0
fi

echo "apt: that needed a fresher index" >&2
sudo timeout 240 apt-get update || echo "apt: update did not finish; trying the install anyway" >&2
install "$@"
