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
# Five seconds and no retries.
#
# Fifteen with two retries was the first attempt, and it is still far too generous: the runner
# image lists about twenty index files, and a mirror that hangs rather than refusing costs
# 20 × 15 × 3 = fifteen minutes before apt gives up on it. `playwright install --with-deps` was
# still sitting there thirty-four minutes in.
#
# There is nothing to lose by being impatient. A mirror that cannot answer in five seconds is not
# one worth waiting for, and every package these workflows need is already in the index the image
# shipped with — the network is the fallback here, not the path.
sudo tee /etc/apt/apt.conf.d/99-summo-timeouts > /dev/null <<'CONF'
Acquire::http::Timeout "5";
Acquire::https::Timeout "5";
Acquire::ftp::Timeout "5";
Acquire::http::ConnectionAttemptDelayMsec "100";
Acquire::Retries "0";
CONF

# Configuration only. Called this way before `playwright install --with-deps`, which installs its
# own list of packages and cannot be handed our flags.
[[ $# -gt 0 ]] || {
  echo "apt: timeouts configured"
  exit 0
}

# `Acquire::Retries` is set to zero above and stays there, but not for this.
#
# Zero is right for the *index*: twenty files against a mirror that hangs, where every retry is
# twenty more timeouts and the index the image shipped with is already good enough. It is wrong for
# the package itself, which is one file of a hundred kilobytes — and a release died twice on
# exactly that, twenty minutes apart, with `ports.ubuntu.com` refusing the connection for
# `libasound2-dev` on arm64 while every other job in the run succeeded. One archive host having a
# bad few minutes should not cost a platform its build.
#
# Bounded by the same impatience as everything else here: three attempts, five seconds of
# connection timeout each, ten seconds between them. Worst case is under a minute, against a
# rebuild-and-rerun that costs twenty.
install() {
  for attempt in 1 2 3; do
    if sudo timeout 900 apt-get -o Acquire::Retries=2 install -y "$@"; then
      return 0
    fi
    [[ $attempt -lt 3 ]] || return 1
    echo "apt: attempt $attempt did not fetch everything; waiting to try again" >&2
    sleep 10
  done
}

# The index the image came with. Usually enough, and it costs nothing to find out.
if install "$@"; then
  echo "apt: installed from the index the image shipped with"
  exit 0
fi

echo "apt: that needed a fresher index" >&2
sudo timeout 120 apt-get update || echo "apt: update did not finish; trying the install anyway" >&2
install "$@" && exit 0

# Last resort: a second archive host, added rather than substituted.
#
# The objection recorded above — that rewriting the sources throws away the cached index, which is
# the one thing on this machine that reliably works — is about doing it *first*. By here the index
# has already been refreshed and the install has already failed nine times against the host it
# names, so there is no fast path left to protect.
#
# This is what took v0.15.0's arm64 bundle down: `ports.ubuntu.com` refused every connection for a
# hundred-kilobyte `libasound2-dev` across three reruns over half an hour, while all eight other
# jobs in the release succeeded. Retrying a host that is out does not help; asking a different one
# does. The file is removed again either way, so nothing about this machine's apt outlives the
# install.
FALLBACK=/etc/apt/sources.list.d/99-summo-fallback.sources
cleanup() { sudo rm -f "$FALLBACK"; }
trap cleanup EXIT

# Ports for everything that is not x86; the main archive for everything that is. The runner tells
# us which it is, and naming the wrong one costs a pointless `apt-get update`.
case "$(dpkg --print-architecture)" in
  amd64 | i386) URI=http://azure.archive.ubuntu.com/ubuntu ;;
  *) URI=http://azure.ports.ubuntu.com/ubuntu-ports ;;
esac

echo "apt: the archive is not answering; adding $URI" >&2
sudo tee "$FALLBACK" > /dev/null <<CONF
Types: deb
URIs: $URI
Suites: $(. /etc/os-release && echo "$VERSION_CODENAME") $(. /etc/os-release && echo "$VERSION_CODENAME")-updates
Components: main universe
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
CONF

sudo timeout 180 apt-get update || echo "apt: the fallback index did not finish either" >&2
install "$@"
