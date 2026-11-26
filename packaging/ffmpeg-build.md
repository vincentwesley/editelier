# Shipped FFmpeg build (D3)

The dev build from `scripts/fetch-ffmpeg.ps1` (BtbN `lgpl-shared`) is **dev only**: it contains FFmpeg's software `h264`/`hevc`/`aac` codecs, and `cargo xtask accept A-LEGAL-01` fails on it on purpose. Releases ship our own build, made with BtbN's scripts at a pinned commit plus the flags below. Status: **not built yet** (`packaging/build-ffmpeg.sh` is an untested stub).

## Recipe
- Base: [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) at a pinned commit, target `win64`, variant `lgpl-shared`, FFmpeg `8.1` (same branch as the dev pin). Needs Docker on Linux or WSL2. MSYS2 or a plain mingw-w64 cross build is the fallback if Docker isn't available; use the same configure line.
- Licence: BtbN's `lgpl` variant passes `--enable-version3` (its `LICENSE.txt` is LGPLv3). For LGPLv2.1 like the About box text (D13), drop `--enable-version3` and any library that needs it. If a v3-only library is kept, ship LGPLv3 and update the About text and `THIRD_PARTY_NOTICES.md` to match.
- Flags appended to BtbN's configure line:

```
--disable-decoder=h264,hevc,aac,aac_fixed,aac_latm
--disable-encoder=aac,h264_amf,h264_mf,h264_nvenc,h264_qsv,h264_vaapi,hevc_amf,hevc_mf,hevc_nvenc,hevc_qsv,hevc_vaapi
--disable-parser=h264,hevc,aac,aac_latm
--disable-hwaccel=h264_d3d11va,h264_d3d11va2,h264_d3d12va,h264_dxva2,h264_nvdec,h264_vulkan,hevc_d3d11va,hevc_d3d11va2,hevc_d3d12va,hevc_dxva2,hevc_nvdec,hevc_vulkan
--disable-ffplay
```

  H.264/HEVC/AAC decode and encode go only through Media Foundation (D3). Check that `ffprobe` still reports H.264/HEVC/AAC streams from container metadata once the parsers are gone (`crates/editelier-media` probe tests); if probing regresses, keep the parsers and record the decision in `docs/DECISIONS.md`. Parsers are not codec implementations, but keeping them is a choice we make on purpose.
- Record: `ffmpeg -hide_banner -buildconf` output, BtbN commit, FFmpeg tag and toolchain go into `BUILD-FFMPEG.txt`. `cargo xtask package` writes it into `licences/FFmpeg/` from whatever is in `third_party/ffmpeg/`.
- Source: attach the FFmpeg tarball for the tag, the BtbN scripts at the commit, and this flag list to the same GitHub Release (LGPL §6, `docs/LEGAL-REVIEW.md` §2).
- Gate: `cargo xtask accept A-LEGAL-01` must pass on the staged `target/msix/layout/ffmpeg.exe` before any release.
