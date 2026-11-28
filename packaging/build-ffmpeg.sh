#!/usr/bin/env bash
# Builds the shipped FFmpeg (D3, packaging/ffmpeg-build.md): LGPL-2.1-or-later, shared, win64,
# cross-compiled with mingw-w64 on Ubuntu 24.04 (WSL2). One command, from the repo root in WSL:
#   bash packaging/build-ffmpeg.sh
# Output: third_party/ffmpeg-lgpl/ (bin/ffmpeg.exe, ffprobe.exe, DLLs, LICENSE.txt, PINNED.txt)
# and third_party/ffmpeg-lgpl-src/ (source archives + this script for the GitHub Release, LGPL §6).
# Both are gitignored. Needs root for apt (prints the command otherwise).
set -euo pipefail

FFMPEG_VER=8.1.3
FFMPEG_SHA=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
DAV1D_VER=1.5.5   # BSD-2-Clause, AV1 decode
DAV1D_SHA=ca96b1d0ba3f01f55de7231d775892b64994ba061aaeeaaa4f662a390b3abf9e
ZLIB_VER=1.3.1    # zlib licence, PNG
ZLIB_SHA=9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23
PKGS="gcc libc6-dev gcc-mingw-w64-x86-64-win32 binutils-mingw-w64-x86-64 mingw-w64-x86-64-dev nasm meson ninja-build pkg-config make xz-utils curl ca-certificates"

repo="$(cd "$(dirname "$0")/.." && pwd)"
self="$repo/packaging/build-ffmpeg.sh"
out="$repo/third_party/ffmpeg-lgpl"
src_out="$repo/third_party/ffmpeg-lgpl-src"
work="${WORK:-$HOME/editelier-ffmpeg}"   # Linux filesystem: building on /mnt/c is slow
host=x86_64-w64-mingw32
cc="$host-gcc-win32"                     # win32 thread model: no libwinpthread DLL
pfx="$work/prefix"
export SOURCE_DATE_EPOCH=1764201600 LC_ALL=C TZ=UTC

missing=$(for p in $PKGS; do dpkg -s "$p" >/dev/null 2>&1 || echo "$p"; done)
if [ -n "$missing" ]; then
  if [ "$(id -u)" = 0 ]; then
    DEBIAN_FRONTEND=noninteractive apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends $missing
  else
    echo "run: sudo apt-get install -y --no-install-recommends $missing" >&2; exit 1
  fi
fi

mkdir -p "$work/dl" "$pfx"
fetch() { # url sha
  local f="$work/dl/${1##*/}"
  [ -f "$f" ] || curl -fsSL --retry 3 -o "$f" "$1"
  echo "$2  $f" | sha256sum -c --quiet - || { rm -f "$f"; echo "SHA-256 mismatch: $1" >&2; exit 1; }
}
fetch "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VER.tar.xz" "$FFMPEG_SHA"
fetch "https://downloads.videolan.org/pub/videolan/dav1d/$DAV1D_VER/dav1d-$DAV1D_VER.tar.xz" "$DAV1D_SHA"
fetch "https://github.com/madler/zlib/releases/download/v$ZLIB_VER/zlib-$ZLIB_VER.tar.gz" "$ZLIB_SHA"
rm -rf "$work/build" "$pfx" && mkdir -p "$work/build" "$pfx"
cd "$work/build"
for f in "$work"/dl/*; do tar xf "$f"; done

# zlib (static)
make -C "zlib-$ZLIB_VER" -f win32/Makefile.gcc PREFIX="$host-" CC="$cc" libz.a >/dev/null
install -Dm644 "zlib-$ZLIB_VER/libz.a" "$pfx/lib/libz.a"
install -Dm644 -t "$pfx/include" "zlib-$ZLIB_VER/zlib.h" "zlib-$ZLIB_VER/zconf.h"

# dav1d (static)
cat > cross.txt <<EOF
[binaries]
c = '$cc'
ar = '$host-ar'
strip = '$host-strip'
windres = '$host-windres'
[host_machine]
system = 'windows'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
EOF
meson setup "dav1d-$DAV1D_VER/b" "dav1d-$DAV1D_VER" --cross-file cross.txt --prefix="$pfx" \
  --libdir=lib --buildtype=release --default-library=static -Denable_tools=false -Denable_tests=false >/dev/null
ninja -C "dav1d-$DAV1D_VER/b" install >/dev/null

# FFmpeg: allow-list build. No gpl/version3/nonfree; no h264/hevc/aac software codecs, parsers or
# hwaccels (Media Foundation handles those, D3); no network. Keep in sync with packaging/ffmpeg-build.md.
CONFIGURE=(
  --target-os=mingw32 --arch=x86_64 --cross-prefix="$host-" --cc="$cc"
  --pkg-config=pkg-config --pkg-config-flags=--static
  --extra-cflags="-I$pfx/include" --extra-ldflags="-L$pfx/lib -static-libgcc"
  --enable-shared --disable-static --disable-debug --disable-doc
  --disable-ffplay --disable-avdevice --disable-network --disable-autodetect --disable-everything
  --enable-w32threads --enable-zlib --enable-libdav1d
  --enable-protocol=file,pipe
  --enable-demuxer=mov,matroska,wav,w64,ogg,flac,mp3,aac,mpegts,avi,image2,png_pipe,jpeg_pipe,rawvideo,pcm_f32le,pcm_s16le
  --enable-muxer=mov,mp4,matroska,webm,wav,ogg,opus,flac,rawvideo,f32le,s16le,null,image2,framemd5
  --enable-decoder=libdav1d,ffv1,vp8,vp9,theora,opus,vorbis,flac,mp3,mp3float,png,mjpeg,rawvideo,'pcm_*'
  --enable-encoder=ffv1,opus,flac,png,rawvideo,wrapped_avframe,'pcm_*'
  --enable-parser=av1,vp8,vp9,opus,vorbis,flac,mpegaudio,png,mjpeg
  --enable-filter=aresample,aformat,anull,null,format,scale,setpts,asetpts,trim,atrim,select,aselect,fps,volume,pad,crop,transpose,hflip,vflip
)
cd "ffmpeg-$FFMPEG_VER"
PKG_CONFIG_LIBDIR="$pfx/lib/pkgconfig" ./configure --prefix="$pfx/ffmpeg" "${CONFIGURE[@]}" | tail -3
make -j"$(nproc)" install >"$work/make.log" 2>&1 || { tail -30 "$work/make.log"; exit 1; }
cd ..

rm -rf "$out" "$src_out" && mkdir -p "$out/bin" "$src_out"
cp "$pfx"/ffmpeg/bin/{ffmpeg,ffprobe}.exe "$pfx"/ffmpeg/bin/*.dll "$out/bin/"
{
  cat "ffmpeg-$FFMPEG_VER/COPYING.LGPLv2.1"
  printf '\n\n----- dav1d %s (statically linked into avcodec), BSD-2-Clause -----\n\n' "$DAV1D_VER"
  cat "dav1d-$DAV1D_VER/COPYING"
  printf '\n\n----- zlib %s (statically linked), zlib licence -----\n\n' "$ZLIB_VER"
  sed -n '/Copyright (C)/,/madler@alumni.caltech.edu/p' "zlib-$ZLIB_VER/zlib.h"
} > "$out/LICENSE.txt"
printf 'ffmpeg-%s LGPLv2.1 shared (packaging/build-ffmpeg.sh; dav1d %s, zlib %s; %s)\n' \
  "$FFMPEG_VER" "$DAV1D_VER" "$ZLIB_VER" "$($cc --version | head -1)" > "$out/PINNED.txt"
cp "$work"/dl/* "$self" "$src_out/"
grep 'define FFMPEG_CONFIGURATION' "ffmpeg-$FFMPEG_VER/config.h" > "$src_out/CONFIGURE.txt"

# Guard: the DLLs import only Windows system DLLs and each other.
deps=$("$host-objdump" -p "$out"/bin/* | sed -n 's/^\s*DLL Name: //p' | sort -u)
echo "$deps" | grep -viE '^(av[a-z]+|sw[a-z]+)-[0-9]+\.dll$|^(kernel32|user32|advapi32|bcrypt|ole32|shell32|ws2_32|msvcrt|api-ms-win-.*)\.dll$' \
  && { echo "unexpected DLL imports (above)" >&2; exit 1; }
echo "FFmpeg $FFMPEG_VER -> $out; sources -> $src_out"
