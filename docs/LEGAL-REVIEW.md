# Legal review (pre-release checklist)

Run before every public release (GitHub Release or Store submission). Engineering checklist, not legal advice. Automatable items map to `A-LEGAL-*` in `docs/ACCEPTANCE.md`. Root rules: `../CLAUDE.md` §8 and `_house/pre-public/CHECKLIST.md`.

## 1. Licences
- [ ] `LICENSE` = MIT, "Copyright (c) 2026 Vin's Atelier" (D12).
- [ ] `NOTICE.md` keeps the logo and creator art **all rights reserved**, with the artist credit (Portus Workstation, @portus_workstation / @wason25_pz).
- [ ] `cargo deny check licenses` + npm licence scan clean (A-LEGAL-02). No GPL/AGPL code; LGPL only as DLLs or a helper process (D11).
- [ ] Vendored FilmCraft crates taken under MIT, file headers intact, PROVENANCE with commit `2d1d1b6` ([R01 §10](research/01-filmcraft.md)).
- [ ] Any OpenReel/OpenCut code: per-file provenance checked, MIT header kept ([R02](research/02-oss-editors.md)).
- [ ] MediaBunny/symphonia (MPL-2.0), if used: unmodified, source pointer in notices.

## 2. FFmpeg LGPL compliance ([ffmpeg.org/legal](https://ffmpeg.org/legal.html), [R04 §7](research/04-windows-media-legal.md))
- [ ] Our own build, no `--enable-gpl`/`--enable-nonfree`; software h264/hevc/aac disabled (A-LEGAL-01).
- [ ] Shared DLLs, original names, beside the exe.
- [ ] `COPYING.LGPLv2.1` + bundled lib licences (dav1d BSD-2, zlib; both in the build's `LICENSE.txt`) in the package and in `THIRD_PARTY_NOTICES.md`.
- [ ] Exact corresponding source (`third_party/ffmpeg-lgpl-src/`: FFmpeg, dav1d, zlib tarballs + `build-ffmpeg.sh` + `CONFIGURE.txt`; no patches) attached to the same GitHub Release.
- [ ] `BUILD-FFMPEG.txt` (configure line, toolchain, versions) shipped.
- [ ] About box credits FFmpeg under LGPLv2.1 with links (D13). Spelled "FFmpeg".
- [ ] Licence terms / Store terms say FFmpeg is LGPLv2.1, not ours, and don't forbid reverse engineering of it.
- [ ] Users can swap the DLLs: portable ZIP on GitHub Releases + documented steps (MSIX installs are read-only; interpretation unverified).
- [ ] Repeat on every FFmpeg bump.
- **Status (2026-11-27):** our own LGPLv2.1 build exists: `packaging/build-ffmpeg.sh` (WSL2, mingw-w64) → `third_party/ffmpeg-lgpl/`, FFmpeg 8.1.3 + dav1d 1.5.5 (BSD-2) + zlib 1.3.1 static, allow-list config with no h264/hevc/aac codecs/parsers/hwaccels and no network (`packaging/ffmpeg-build.md`). A-LEGAL-01 passes on it. `cargo xtask package` bundles it when present (falls back to the dev BtbN LGPLv3 build with a warning; never publish that MSIX). Source archive for the Release: `third_party/ffmpeg-lgpl-src/`.
- Package layout: `licences/FFmpeg/LICENSE.txt` (the build's own licence text) and `licences/FFmpeg/BUILD-FFMPEG.txt` (`-buildconf` + pin + source links) are written by `cargo xtask package`.

## 3. Codec patents ([R04 §3](research/04-windows-media-legal.md))
- [ ] H.264, HEVC, AAC encode/decode only via Windows Media Foundation (D3). Our binaries contain no implementation of them.
- [ ] HEVC: detect the Store extension and link to it; never bundled.
- [ ] Royalty-free export path (AV1 + Opus) works.
- [ ] **Open:** ProRes / DNx decode patent status verified before enabling.
- Stance: relying on OS codecs is industry practice (Firefox precedent), not a written safe harbour. Revisit with counsel if the app ever turns commercial.

## 4. AI models ([R04 §5](research/04-windows-media-legal.md))
- [ ] Each shipped model: licence file shipped, source URL + SHA-256 in manifest, row in notices (A-LEGAL-06).
- [ ] Whisper / distil-whisper: MIT. MODNet: upstream Apache-2.0 weights only. MediaPipe Selfie: Google original only (not Qualcomm repackages).
- [ ] BiRefNet: LICENSE file read and confirmed MIT before inclusion.
- [ ] RMBG-1.4/2.0 absent (non-commercial).
- [ ] ONNX Runtime (MIT) + DirectML redistributable terms checked from the NuGet licence.

## 5. Bundled content
- [ ] Fonts: OFL-1.1 with `OFL.txt` beside them (house fonts already listed).
- [ ] Icons/pixel art: CC0/MIT per `THIRD_PARTY_NOTICES.md`.
- [ ] Any sample media / LUTs / templates: CC0 or own, credited. No Pexels, no stock figures, no AI-generated art.
- [ ] Test media is not in the package; fetched corpus stays gitignored.

## 6. Trademarks and clean room
- [ ] No Premiere/DaVinci/Resolve/CapCut in name, icon, Store listing, screenshots or marketing (D19). Keymap names descriptive only, with "not affiliated" note in Settings.
- [ ] No proprietary icons, layouts, colour schemes or trade dress.
- [ ] No ArtCraft/FilmCraft names or owl icon.
- [ ] **Name check status for "Editelier"** ([R04 §6](research/04-windows-media-legal.md)):

| Register | Status |
|---|---|
| USPTO | 0 hits (2026-11-25) |
| Microsoft Store | 0 results (API) |
| GitHub, npm, PyPI, crates.io | unclaimed |
| Web | no exact product |
| editelier.com | registered by third party, no site |
| **IPOS (Singapore)** | **manual, owner (NEEDS-VIN)** |
| **WIPO Global Brand DB** | **manual, owner** |
| **EUIPO TMview** | **manual, owner** |

## 7. Privacy
- [ ] `docs/PRIVACY.md` true for this build: no network calls, no telemetry (A-LEGAL-05).
- [ ] Privacy policy published at a public URL (landing page) for the Store (policy 10.5.1).
- [ ] Automation channel absent from release (A-LEGAL-04).

## 8. Microsoft Store policy ([policies v7.20](https://learn.microsoft.com/en-us/windows/apps/publish/store-policies))
- [ ] Unique title, no keywords (10.1).
- [ ] Declared capabilities match features: `runFullTrust` only (10.6).
- [ ] No dynamic code changing functionality (10.2.2); model files are data.
- [ ] IARC questionnaire done (11.11).
- [ ] Description notes on-device AI (transcription, segmentation) and FFmpeg use.
- [ ] Identity fields copied from Partner Center.

## 9. Notice files
- [ ] `THIRD_PARTY_NOTICES.md` lists only what's actually shipped, each with licence + source (A-LEGAL-03).
- [ ] `NOTICE.md`, `LICENSE`, `SECURITY.md`, README "provided as is" present.
- [ ] Notices reachable in-app (About → Licences).
