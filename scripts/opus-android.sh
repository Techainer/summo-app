#!/usr/bin/env bash
#
# Build Opus for Android, because the crate that bundles it cannot cross-compile.
#
# `audiopus_sys` 0.1.8 builds its vendored Opus by running `configure` with no `--host`. Autoconf
# therefore believes it is building for the machine it is running on: it compiles a test program
# with the NDK's compiler, tries to *run* it, and stops with "cannot run C compiled programs". Set
# the compiler and leave the flag off and it goes one step further and produces an x86-64
# `libopus.so`, which the linker then rejects with "incompatible with aarch64linux". Both failures
# are the same missing argument.
#
# The crate has a supported way out: `OPUS_LIB_DIR` skips the bundled build entirely and links
# whatever is in that directory. So this builds Opus properly, once, and the Android build points at
# it. Nothing here is patched or vendored — it is the upstream release, configured the way
# cross-compiling an autotools project has always been done.
set -euo pipefail

VERSION="${OPUS_VERSION:-1.5.2}"
ABI="${1:-arm64-v8a}"
API="${ANDROID_API:-26}"
OUT="${OPUS_PREFIX:-$HOME/.cache/summo/opus-android/$VERSION/$ABI}"

case "$ABI" in
  arm64-v8a)   HOST=aarch64-linux-android;   CC_PREFIX=aarch64-linux-android ;;
  armeabi-v7a) HOST=armv7a-linux-androideabi; CC_PREFIX=armv7a-linux-androideabi ;;
  x86_64)      HOST=x86_64-linux-android;    CC_PREFIX=x86_64-linux-android ;;
  *) echo "unknown ABI: $ABI" >&2; exit 2 ;;
esac

: "${NDK_HOME:?set NDK_HOME to the Android NDK}"
TOOLS="$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
[ -d "$TOOLS" ] || TOOLS="$NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin"

if [ -f "$OUT/lib/libopus.a" ]; then
  echo "opus: already built at $OUT"
  echo "$OUT/lib"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "opus: fetching $VERSION"
curl -fsSL "https://downloads.xiph.org/releases/opus/opus-$VERSION.tar.gz" | tar xz -C "$WORK"

cd "$WORK/opus-$VERSION"
export CC="$TOOLS/${CC_PREFIX}${API}-clang"
export AR="$TOOLS/llvm-ar"
export RANLIB="$TOOLS/llvm-ranlib"
export STRIP="$TOOLS/llvm-strip"

# `--host` is the whole point: it is what tells autoconf not to try running what it just compiled.
# Static, because a shared library would have to be packaged into the APK and found at runtime, and
# there is no reason to ship a second file for eighteen kilobytes of codec.
./configure \
  --host="$HOST" \
  --prefix="$OUT" \
  --enable-static \
  --disable-shared \
  --disable-doc \
  --disable-extra-programs \
  --with-pic \
  >/dev/null

make -j"$(nproc)" >/dev/null
make install >/dev/null

echo "opus: built $OUT/lib/libopus.a"
echo "$OUT/lib"
