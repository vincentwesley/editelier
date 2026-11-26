# Acceptance checks

Every check is scriptable and has a stable id. Runner (planned, M1): `cargo xtask accept <id>`, `cargo xtask accept --milestone m1`, `cargo xtask accept --all`. Results go to `qa-output/accept/<id>.json` (gitignored). Oracle = the local ffmpeg/ffprobe build (GPL, with libvmaf; test-only, never shipped; version pinned in `xtask/README.md`). UI flows drive the app through WebdriverIO + `@wdio/tauri-service` (D8) and native state through the debug automation channel (D7).

Perf ids run on the reference laptop (i5-1335U / Iris Xe / 16 GB, plugged in, balanced power plan, release build). Milestone mapping: `docs/PLAN.md`. Ids for M2+ not listed here get thresholds when their milestone starts.

Fixtures: `F-*` are self-generated and committed (`test-media/synthetic/`); `C-*` are fetched corpus files (see Import corpus).

## A-CUT: frame exactness and editing

Method: per timeline frame, the debug command `render_frame_hash --stage decoded` returns the MD5 of the decoded source planes (NV12/P010, before colour conversion) and is compared to `ffmpeg -i <ref> -pix_fmt nv12 -f framemd5` of an ffmpeg-made reference cut. Exports use the test-only lossless profile (FFV1/MKV, debug builds) for framemd5; MP4 exports are checked by `ffprobe -count_frames` frame count and PTS list.

| Id | Check | Pass |
|---|---|---|
| A-CUT-01 | Trim in/out on F-cfr30-longgop (GOP 250) at 20 random frame pairs | framemd5 identical; frame count exact |
| A-CUT-02 | Split at frame N on F-bframes-editlist (2 B-frames, `elst`, AAC priming) | both halves framemd5-identical to reference; no duplicated/dropped frame at the seam |
| A-CUT-03 | Join two clips from different files (same format) | framemd5 identical across the join; audio sample count = sum of parts ± 0 |
| A-CUT-04 | Ripple delete a middle segment | downstream frames shift by exactly the deleted length; framemd5 identical |
| A-CUT-05 | VFR: F-vfr-screen (PTS jitter) + C-phone-vfr on a 30p timeline | timeline frame t = latest source frame with PTS ≤ t (index check); CFR export A/V offset ≤ 5 ms at start and end of a 10 min fixture |
| A-CUT-06 | Undo/redo: seeded random sequence of 500 commands | undo all → project JSON byte-equal to initial; redo all → byte-equal to final; no panic |
| A-CUT-07 | Seek accuracy: 20 random seeks per fixture (CFR, VFR, B-frames, edit list, HEVC 10-bit, HLG) | returned PTS = `ffprobe -show_frames` PTS for 100% of seeks |

## A-QUAL: output quality and conformance

| Id | Check | Pass |
|---|---|---|
| A-QUAL-01 | 1080p "High" preset export vs lossless render of the same timeline (`libvmaf`, `vmaf_v0.6.1`, `float_ssim`) | VMAF mean ≥ 93, 1st-percentile ≥ 85, SSIM ≥ 0.97; mean drop vs stored baseline ≤ 2 |
| A-QUAL-02 | Same at 4K | same thresholds |
| A-QUAL-03 | Golden frames (offscreen render, fixed seed/resolution): title, fades, scaling, 16:9/9:16 canvas | per-pixel max Δ ≤ 2/255 on ≥ 99.9% of pixels and SSIM ≥ 0.995 per frame; failure writes actual + ×8 diff |
| A-QUAL-04 | Colour conversion (CPU reference): F-tags-{601,709}×{limited,full} | max Δ ≤ 1 code value per channel vs expected RGB |
| A-QUAL-05 | HDR HLG + PQ fixtures tone-mapped to SDR | golden thresholds of A-QUAL-03; output tagged BT.709 |
| A-QUAL-06 | Loudness normalise (oracle: ffmpeg `ebur128=peak=true`) | Social: −14 ± 0.5 LUFS, TP ≤ −1.0 dBTP; Broadcast (M3): −23 ± 0.5 LUFS, TP ≤ −1.0 dBTP |
| A-QUAL-07 | Export conformance per preset (ffprobe JSON) | resolution/fps/codec/profile per preset file; zero-based PTS; no `elst`; `moov` before `mdat`; closed GOP; AAC-LC 48 kHz stereo; `color_primaries/transfer/space = bt709`, `color_range = tv` |

## A-PERF: budgets (reference laptop)

Measured by the headless bench (`cargo xtask accept` drives playback offscreen via `tracing` timings; PresentMon for on-screen drops).

| Id | Check | Pass |
|---|---|---|
| A-PERF-01 | 1080p30 H.264 timeline (3 cuts + title + music) playback, 60 s | 0 dropped frames (PresentMon `MsBetweenDisplayChange`), render p95 ≤ 16 ms |
| A-PERF-02 | 4K30 and 4K60 HEVC sources via proxy | ≥ 30 fps sustained over 60 s |
| A-PERF-03 | 1080p30 export, 60 s timeline, "High" | ≥ 2× realtime (≤ 30 s wall) |
| A-PERF-04 | 4K30 export on QSV, 60 s timeline | ≥ 0.75× realtime (≤ 80 s wall) |
| A-PERF-05 | Cold start to interactive Welcome | < 2 s |
| A-PERF-06 | Import probe per file (corpus mean) | < 300 ms/file; first thumbnail visible < 1 s |
| A-PERF-07 | Scrub to frame | ≤ 100 ms within keyframe interval / ≤ 250 ms long GOP (originals), p95 |

## A-UI: flows (WebdriverIO + automation channel)

| Id | Check | Pass |
|---|---|---|
| A-UI-01 | First run: "What are you making?" → Short | canvas 1080×1920, export preset TikTok/Short selected |
| A-UI-02 | Golden flow: import 2 clips → split → ripple delete → title → music + fade → export 1080p YouTube | file exists, passes A-QUAL-07; frame hashes at 5 checkpoints match expected |
| A-UI-03 | "Coming from" Premiere-style, then Resolve-style | split key changes (Ctrl+K / Ctrl+B); `?` overlay lists the active map |
| A-UI-04 | Undo/redo via menu, keys and History | state after each matches A-CUT-06 expectation |
| A-UI-05 | axe on Welcome, editor, export, About, settings, dark + light | 0 serious/critical |
| A-UI-06 | About dialog | credits card, logo + artist credit, FFmpeg LGPLv2.1 notice with ffmpeg.org, licence and source-archive links |
| A-UI-07 (M3) | Kill process mid-edit, relaunch | project reopens at the last journaled command with a one-line banner |

## A-COL: colour and looks (D20)

Method: offscreen render through the full grade pipeline (input transform → Fix → Look → Film → DRT), fixed seed and resolution, compared like A-QUAL-03 unless stated. Test images: the fixed open set `C-colour` (CC0/CC BY photos + owner-shot street, interior and face clips, no copyrighted reference frames) plus committed synthetic charts (`F-chart-grey`, `F-chart-skin`: grey ramp, colour patches, skin-tone patches along the I-line). Film grain is seeded by frame index.

| Id | Check | Pass |
|---|---|---|
| A-COL-01 (M1) | Neutral grade on SDR (8-bit 709 limited + full, sRGB still) round-trip through inverse DRT → DRT | max Δ ≤ 1/255 per channel on 100 % of pixels vs the source |
| A-COL-02 (M2) | Each built-in look at 100 % on every `C-colour` image + charts | golden frames (A-QUAL-03 thresholds); failure writes actual + ×8 diff |
| A-COL-03 (M2) | Strength 0 % and 100 % per look | 0 % = A-COL-01 identity; 100 % = the A-COL-02 golden; 150 % renders finite (no NaN/Inf), clamped |
| A-COL-04 (M2) | Skin protect on, each look at 100 % on `F-chart-skin` | every skin patch's hue within ±15° of the skin line (OKLCh, measured from the chart); protection off shows the look's raw shift (sanity) |
| A-COL-05 (M2) | Grade cost: 100 % look incl. film layer, GPU timestamps, A-PERF conditions | ≤ 5 ms/frame 1080p p95 on Iris Xe |
| A-COL-06 (M5) | `.cube` parser: fuzz (cargo-fuzz, 10 min at milestone) + malformed corpus (bad size, wrong entry count, NaN/Inf, out-of-domain, > 64 MB, truncated, non-UTF-8) | no panic or OOM; every malformed file rejected with a plain-language error; valid LUTs match a CPU tetrahedral reference within 1/1023 |

## A-IMPORT: corpus

Every corpus file: probe succeeds, thumbnail + waveform produced, VFR flag correct, proxy decision correct, nothing panics; unsupported files show a plain-language error with a fix (for example a Store link for HEVC).

| Id | Scope |
|---|---|
| A-IMPORT-01 | Video corpus (table below) |
| A-IMPORT-02 | Audio corpus |
| A-IMPORT-03 | Stills + RAW in M1: graceful (still as image clip or clear error); full support M4/M9 |
| A-IMPORT-04 (M4), A-IMPORT-05 (M9) | Stills and RAW fully supported |

| Corpus id | Content | Source |
|---|---|---|
| C-phone-hevc | Phone HEVC 10-bit HLG, 4K | owner-shot, no people (NEEDS-VIN) |
| C-phone-vfr | Phone H.264 VFR 1080p | owner-shot |
| C-screen-vfr | Screen recording VFR | self-made (Game Bar / ffmpeg gdigrab) |
| C-4k60 | 4K60 H.264 + HEVC | Blender open movie (CC BY), transcoded locally |
| C-audio | MP3, WAV, AAC/M4A, FLAC, Opus | self-generated `sine` + Blender soundtrack excerpts (CC BY) |
| C-stills | JPEG (EXIF orientations), PNG (alpha), HEIC | self-made + CC0 |
| C-raw | 3–5 RAWs (Canon CR3, Sony ARW, Nikon NEF, DNG) | raw.pixls.us, CC0 checked per file |

Fetched via `cargo xtask fetch-media` from `test-media/MANIFEST.json` (URL, SHA-256, licence, attribution). Never Pexels (D15).

## A-MSIX: packaging

| Id | Check | Pass |
|---|---|---|
| A-MSIX-01 | Manifest lint | `runFullTrust` present; no `broadFileSystemAccess`; `MinVersion` 10.0.22000; file associations present; version bumped |
| A-MSIX-02 | `Add-AppxPackage` with dev cert | installs, exit 0 |
| A-MSIX-03 | Launch from Start (package identity), automation ping, open sample project | responds < 5 s; no writes to install dir |
| A-MSIX-04 | `Remove-AppxPackage` | package gone; no leftovers under `%LOCALAPPDATA%\Packages\<family>` |
| A-MSIX-05 | `dumpbin /dependents` on exe + DLLs | no undeclared runtime dependency (VCRuntime only if a `PackageDependency` is declared) |

## A-LEGAL

| Id | Check | Pass |
|---|---|---|
| A-LEGAL-01 | Shipped FFmpeg: `-buildconf`, `-decoders`, `-encoders` | no `--enable-gpl`/`--enable-nonfree`; no software `h264`, `hevc`, `aac`, `aac_fixed` codecs; `BUILD-FFMPEG.txt` matches |
| A-LEGAL-02 | `cargo deny check licenses` + npm licence scan | only allow-listed licences (MIT, Apache-2.0, BSD-2/3, ISC, Zlib, Unicode, OFL for fonts, MPL-2.0 unmodified); no GPL/AGPL; no statically linked LGPL |
| A-LEGAL-03 | Package file list vs `THIRD_PARTY_NOTICES.md` | every bundled third-party file listed with licence; LGPL text + FFmpeg source link present |
| A-LEGAL-04 | Release exe | 0 matches for `editelier-automation` (automation channel absent) |
| A-LEGAL-05 | Network + naming | no HTTP client crate in `cargo tree` of release; CSP `connect-src` has no remote hosts; "Premiere/DaVinci/Resolve/CapCut" appear only in the keymap names file and docs |
| A-LEGAL-06 (M6+) | Model files | SHA-256 matches manifest; licence file shipped; listed in notices |
