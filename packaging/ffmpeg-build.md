# Shipped FFmpeg build (D3)

The dev build from `scripts/fetch-ffmpeg.ps1` (BtbN `lgpl-shared`, LGPLv3, into `third_party/ffmpeg/`) is **dev only**: it contains FFmpeg's software `h264`/`hevc`/`aac` codecs, and `cargo xtask accept A-LEGAL-01` fails on it on purpose. Releases ship our own build, made by `packaging/build-ffmpeg.sh`.

## Build (one command)
From the repo root inside WSL2 Ubuntu 24.04 (root, or a user with sudo for the apt step; the script prints the `apt-get` line if it can't install):

```
bash packaging/build-ffmpeg.sh
```

- Output: `third_party/ffmpeg-lgpl/` (`bin/ffmpeg.exe`, `bin/ffprobe.exe`, `avcodec-62`, `avformat-62`, `avfilter-11`, `avutil-60`, `swresample-6`, `swscale-9` DLLs; `LICENSE.txt` = LGPLv2.1 + dav1d + zlib notices; `PINNED.txt`) and `third_party/ffmpeg-lgpl-src/` (source tarballs, the script, `CONFIGURE.txt`). Both are gitignored (`/third_party/`). ~11 min on the i5-1335U, ~11 MB total.
- `cargo xtask package` and `cargo xtask accept A-LEGAL-01` use `third_party/ffmpeg-lgpl/` when it exists and fall back to the dev build (with a warning) otherwise. App/tests: `EDITELIER_FFMPEG_DIR=<repo>/third_party/ffmpeg-lgpl/bin`.
- Toolchain: Ubuntu 24.04 mingw-w64 GCC 13 (`x86_64-w64-mingw32-gcc-win32`, win32 thread model: no libwinpthread DLL), `-static-libgcc`. The DLLs import only `KERNEL32`, `SHELL32`, `bcrypt`, `msvcrt` and each other (the script fails otherwise).

## Pins
| Source | Version | SHA-256 | Licence |
|---|---|---|---|
| https://ffmpeg.org/releases/ffmpeg-8.1.3.tar.xz | 8.1.3 | `7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3` | LGPL-2.1-or-later (as configured) |
| https://downloads.videolan.org/pub/videolan/dav1d/1.5.5/dav1d-1.5.5.tar.xz | 1.5.5 | `ca96b1d0ba3f01f55de7231d775892b64994ba061aaeeaaa4f662a390b3abf9e` | BSD-2-Clause (static, AV1 decode) |
| https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz | 1.3.1 | `9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23` | Zlib (static, PNG) |

## Configuration (allow-list)
LGPLv2.1: no `--enable-gpl`, `--enable-version3` or `--enable-nonfree`. `--disable-everything` then only royalty-free components are enabled, so there are no software H.264/HEVC/AAC decoders, encoders, parsers or hwaccels at all (Media Foundation handles those, D3), no network protocols and no avdevice. Probing H.264/HEVC/AAC files still works from container metadata (codec, size, fps, duration, rotation, sample rate, `colr` atom colour), but without a decoder ffprobe reports `pix_fmt`, `profile` and the bitstream (VUI) colour as `unknown`. Tried keeping the `h264`/`hevc`/`aac` parsers (2026-11-27): no change for mp4/mov, so they stay off. Consequence: HEVC 10-bit / HLG/PQ detection must come from Media Foundation's native media type (`MF_MT_VIDEO_PROFILE` / `MF_MT_TRANSFER_FUNCTION` / `MF_MT_VIDEO_PRIMARIES` / `MF_MT_YUV_MATRIX`), not ffprobe; until that lands, `probe_corpus` fails on `hlg10_720p_hevc.mp4` with this build. Exact `ffmpeg -hide_banner -buildconf`:

```
--prefix=/root/editelier-ffmpeg/prefix/ffmpeg
--target-os=mingw32
--arch=x86_64
--cross-prefix=x86_64-w64-mingw32-
--cc=x86_64-w64-mingw32-gcc-win32
--pkg-config=pkg-config --pkg-config-flags=--static
--extra-cflags=-I/root/editelier-ffmpeg/prefix/include
--extra-ldflags='-L/root/editelier-ffmpeg/prefix/lib -static-libgcc'
--enable-shared
--disable-static
--disable-debug
--disable-doc
--disable-ffplay
--disable-avdevice
--disable-network
--disable-autodetect
--disable-everything
--enable-w32threads
--enable-zlib
--enable-libdav1d
--enable-protocol='file,pipe'
--enable-demuxer='mov,matroska,wav,w64,ogg,flac,mp3,aac,mpegts,avi,image2,png_pipe,jpeg_pipe,rawvideo,pcm_f32le,pcm_s16le'
--enable-muxer='mov,mp4,matroska,webm,wav,ogg,opus,flac,rawvideo,f32le,s16le,null,image2,framemd5'
--enable-decoder='libdav1d,ffv1,vp8,vp9,theora,opus,vorbis,flac,mp3,mp3float,png,mjpeg,rawvideo,pcm_*'
--enable-encoder='ffv1,opus,flac,png,rawvideo,wrapped_avframe,pcm_*'
--enable-parser='av1,vp8,vp9,opus,vorbis,flac,mpegaudio,png,mjpeg'
--enable-filter='aresample,aformat,anull,null,format,scale,setpts,asetpts,trim,atrim,select,aselect,fps,volume,pad,crop,transpose,hflip,vflip'
```

What our helper needs (`crates/editelier-media/src/ffmpeg.rs`): ffprobe JSON/CSV probe + packet index, `rawvideo`/`rgba` single-frame decode of the `ROYALTY_FREE` codecs, `aresample` → `f32le` audio. The rest (FFV1, Opus/FLAC encode, mp4/mkv/wav mux) is for the lossless/royalty-free export paths. Adding a codec or filter = add it to `CONFIGURE` in the script, rebuild, update this file, rerun A-LEGAL-01.

## Release obligations
- `cargo xtask package` copies the exes + DLLs (original names) beside `editelier.exe` and writes `licences/FFmpeg/LICENSE.txt` and `licences/FFmpeg/BUILD-FFMPEG.txt` (`PINNED.txt` + `-buildconf`).
- Attach everything in `third_party/ffmpeg-lgpl-src/` (FFmpeg, dav1d, zlib tarballs, `build-ffmpeg.sh`, `CONFIGURE.txt`) to the same GitHub Release (LGPL §6, `docs/LEGAL-REVIEW.md` §2).
- Gate: `cargo xtask accept A-LEGAL-01` must pass on the staged `target/msix/layout/ffmpeg.exe` before any release (it also checks `BUILD-FFMPEG.txt` against `-buildconf` there).
