# Decisions

Owner-approved 2026-11-25. Each entry: decision, why, research link. Research files: `docs/research/0N-*.md` (abbreviated R01..R06). Change a decision by adding a new numbered entry that supersedes it; don't rewrite history.

## D1 Stack and preview surface
- **Decision:** Tauri 2 shell + TypeScript/Vite UI (house style kit), Rust Cargo workspace core. Preview = wgpu on DX12 in a native **child HWND** positioned (via `SetWindowPos`) under a rectangle the WebView2 UI reserves and reports (ResizeObserver + `devicePixelRatio`). All viewer overlays (safe areas, crop handles, text gizmos, scopes) are drawn in wgpu; no HTML over the viewer.
- **First task of M1:** a spike proving it (resize, DPI change, modal dialogs, 4K-proxy playback, zero-copy decode→present).
- **Fallback (if the spike fails):** native egui/wgpu preview pane window sharing the wgpu device (the path FilmCraft proves works).
- **Why:** zero-copy decode→composite→present is the only path that isn't bus-bound on Iris Xe; frame streaming to `<canvas>` costs ~250 MB/s at 1080p30; drawing over WebView2 has airspace/overlay-plane bugs. Web UI keeps the house kit and accessibility.
- **Research:** R04 §1, R01 §2 + verdict, R02 (OpenTake uses the same stack).

## D2 FilmCraft: reuse crates, don't fork
- **Decision:** Don't fork FilmCraft. **Vendor** (copy into `crates/vendor/`, not git deps) under its **MIT** option, file copyright headers kept, a `PROVENANCE` note per crate (source URL + commit) and a row each in `THIRD_PARTY_NOTICES.md` when they land:
  - `filmcraft-audio-dsp` (zero deps; R128/BS.1770, EQ, limiter)
  - `filmcraft-color` (transfer functions, YUV matrices, BT.2390, LUTs)
  - `filmcraft-scopes` (waveform, parade, histogram, vectorscope)
  - the tick time type: 254,016,000,000 ticks/s `Tick(i64)` + rational `FrameRate`, from `storytold/craft-libs` `craft-timeline` at the revision FilmCraft `2d1d1b6` pins.
- Source commit: FilmCraft `2d1d1b6`. Vendored copies chosen over git pins for stability (upstream releases every 2–3 days, recently moved code to craft-libs).
- **Copy patterns (own code):** command registry shared by UI/CLI/agent (id, param schema, `enabled()` with reason, undo journal); copy-on-write `Arc<Project>` undo; ffmpeg-oracle + golden-frame + GPU-parity tests; layering check in xtask.
- **Evaluate (M3+):** OpenCut classic MIT wgpu crates (`gpu`/`compositor`/`effects`/`masks`/`time`, wgpu 29) for compositor/effects; pin a commit if used.
- Never ship ArtCraft/FilmCraft names or icons. Avoid `symphonia` (MPL-2.0) unless we accept the source-pointer duty.
- **Why:** wrong product shape (Premiere clone, egui UI that upstream won't swap for Tauri), Windows is their weakest platform, high churn. The pure crates are high value, low coupling.
- **Research:** R01 verdict + §10, R02 §A/Recommendations.

## D3 Media: Media Foundation for patented codecs, LGPL FFmpeg for the rest
- **Decision:** H.264, HEVC and AAC **decode and encode only through Windows Media Foundation** (hardware MFTs QSV/AMF/NVENC first, Microsoft software MFT fallback; D3D11 output, zero-copy into wgpu via shared NT handles + fence). FFmpeg 8.x **LGPL, shared DLLs**, via `rsmpeg`, for probe, demux/mux, VFR/timestamp index, and royalty-free codecs (AV1 incl. dav1d, VP8/VP9, Opus, FLAC, PCM, PNG, MJPEG).
- **FFmpeg build:** BtbN `lgpl-shared` builds still contain FFmpeg's native `h264`/`hevc`/`aac` decoders, so the **shipped** FFmpeg is our own LGPL build (BtbN scripts at a pinned tag) with `--disable-decoder=h264,hevc,aac,aac_fixed` and the matching encoders/parsers we don't need disabled; `ffmpeg -buildconf` recorded in `BUILD-FFMPEG.txt`. A pinned BtbN `lgpl-shared` tag may be used for local development only.
- **Exports:** default H.264 + AAC MP4 via MF; royalty-free option AV1 + Opus via FFmpeg (HW AV1 where present, else SVT-AV1/rav1e).
- **HEVC:** needs the user's Store "HEVC Video Extensions": detect, show a Store link, never bundle.
- **Open question:** ProRes/DNx decode patent status (verify before enabling those decoders).
- **Layering:** MF code lives behind the `media` trait in `media-win`; the editing core is platform-free.
- **Why:** pools license whoever ships a codec implementation; calling OS codecs is the Firefox precedent (industry practice, not a written safe harbour). Note R04 recommended FFmpeg `d3d11va` decode; we chose MF because d3d11va still uses libavcodec's H.264/HEVC parsing code (grey zone).
- **Research:** R04 §2, §3, §7; R02 §C (rsmpeg).
- **Amendment (2026-11-26, M1 build findings):**
  - **FFmpeg runs as LGPL helper processes** (`ffprobe.exe` / `ffmpeg.exe` beside the app) instead of in-process `rsmpeg`: no libclang build dependency, and the process boundary makes LGPL compliance and user replacement trivial (swap the exes), which also resolves the MSIX DLL-swap question (D9, `docs/LEGAL-REVIEW.md`). In-process can be revisited if helper latency proves too high (for example per-frame demux in playback).
  - **MF export limits:** MF's moov-first (faststart) option corrupts output, so we relocate `moov` ourselves after the write (current implementation limited to files ≤ 4 GiB; lift via `co64` before long 4K exports). MF H.264 exports carry no B-frames. MF's AAC encoder tops out at **192 kbps**: that is our ceiling, even where a platform preset asks for more (YouTube recommends 384 kbps stereo). MF has no lossless H.264, so the MF test export profile is H.264 CQP 12 (near-lossless, not bit-exact). Frame-exact cut checks (A-CUT) instead export losslessly to FFV1/MKV through the LGPL FFmpeg helper (royalty-free, test/archival path only), then compare with `framemd5`.

## D4 Timing, VFR, seeking, proxies
- **Decision:** all time in integer ticks (D2 type), rational frame rates, half-open ranges, floats only at the UI edge. VFR is **kept** on the timeline: timeline time t shows the latest source frame with PTS ≤ t (sample-and-hold), "VFR" badge on import; conform to CFR **only at export**, audio as master clock. Frame-exact seek via our own per-file keyframe/PTS index (seek to keyframe ≤ target, decode forward, discard until PTS ≥ target). Edit lists honoured on import; exports write zero-based PTS, no edit lists, `+faststart`, closed GOP.
- **Proxies:** auto-generated on import (background) for sources >1080p or long-GOP/VFR; short-GOP (≤1 s), 1080p, in a disk cache keyed by file hash under `%LOCALAPPDATA%` (MSIX: LocalCache); one "use originals" toggle; export always reads originals.
- **Why:** MF seeking isn't frame-exact and passes VFR timestamps through; Premiere/Resolve force users to transcode VFR first. R03 suggested silent conform on import; R05's preserve-then-conform keeps every source frame and is cheaper, so we take that.
- **Research:** R05 §A1–A2, R04 §2 (MF gaps), R01 §4, R03 §3.

## D5 Colour
- **Decision:** linear-light Rec.709 working space, 16-bit float (`Rgba16Float`) buffers; Y'CbCr→RGB on GPU using the source's tagged matrix and range (fallback: ≤576 lines → 601, else 709); HDR HLG/PQ tone-mapped to SDR (BT.2390, own WGSL); preview via an sRGB swapchain; exports tagged BT.709 primaries/transfer/matrix, **limited range**.
- **Why:** correct blending/scaling; YouTube converts to BT.709 limited anyway.
- **Research:** R05 §A3, R01 §3.

## D6 Audio
- **Decision:** 48 kHz f32 internal mix (resample on import with `rubato`), `cpal` output clock drives the playhead, `ebur128` crate (MIT) for integrated LUFS + true peak. Export normalise presets: **−14 LUFS / −1 dBTP (Social)**, **−23 LUFS / −1 dBTP (Broadcast, EBU R128)**; two-pass measure → gain → true-peak limiter only if needed.
- **Note:** −14 LUFS for YouTube/TikTok/Instagram is a convention, not a published platform spec.
- **Research:** R05 §A4, R02 §C, R01 §2 (audio clock).

## D7 Agent control channel
- **Decision:** our own JSON-RPC over a per-user Windows named pipe, compiled only in debug builds with an `agent` cargo feature (a release build with it fails to compile), enabled at runtime by `EDITELIER_AGENT=1`. Code: `src-tauri/src/agent.rs`. Pipe `\\.\pipe\editelier-agent-<random>`, `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`, DACL = current user only; per-session 256-bit token in a user-only file deleted on exit; fixed command allow-list; strict serde input structs; test-workspace path confinement. Used by tests and walkthroughs. Never use WebView2 `--remote-debugging-port`.
- **Optional:** evaluate the P3GLEG Tauri MCP plugin (token auth, refuses release builds) in dev only.
- **Why:** tests need native state (frame hashes, timeline) WebDriver can't see; FilmCraft's MCP lacked path confinement despite its own rule.
- **Research:** R05 §C3, R01 §7.

## D8 E2E testing
- **Decision:** WebdriverIO + `@wdio/tauri-service` (auto-downloads the matching msedgedriver) + `@axe-core/webdriverio`. The app exposes a debug-only command returning preview frame hashes, because WebDriver sees only the DOM, not the wgpu viewer. WebView2 can't run headless, so E2E runs at milestones only; fast a11y checks run on the Vite-served UI.
- **Research:** R05 §B5–B6.

## D9 Packaging and distribution
- **Decision:** MSIX built with Microsoft **winapp CLI** (tauri-bundler has no MSIX). Self-signed dev cert for local installs (never committed); the Store re-signs on submission. Manifest: `Windows.FullTrustApplication` + `runFullTrust`, **no** `broadFileSystemAccess`, `MinVersion` Windows 11 (10.0.22000), file associations (.mp4/.mov/.mkv/.jpg/.png/project file), identity copied from Partner Center. Caches in LocalCache, install dir read-only.
- **Distribution:** Microsoft Store (free), unsigned MSIX + test-cert instructions and a portable ZIP (lets users swap the FFmpeg helper exes, see LGPL and the D3 amendment) on GitHub Releases for testers, landing page on Cloudflare Pages (D18).
- **Research:** R04 §4, §7 item 10.

## D10 On-device AI
- **Decision:** captions via whisper.cpp through `whisper-rs` with Whisper (MIT) / distil-whisper (MIT, English-only) models. Background removal via `ort` + DirectML with MODNet or MediaPipe Selfie (Apache-2.0, Google original only), or BiRefNet(-lite) only after its LICENSE file is verified MIT. **RMBG-1.4/2.0 excluded** (non-commercial). Model files pinned with SHA-256; each listed in THIRD_PARTY_NOTICES. EP choice behind an interface (DirectML is in sustained engineering; Windows ML later).
- **Research:** R04 §5, R02 §C.

## D11 RAW / HEIC
- **Decision:** prefer Windows WIC codecs (Raw Image Extension, HEIF + HEVC extensions). LGPL libraries (LibRaw, rawler, libheif) only as separate DLLs or a helper process, never statically linked.
- **Research:** R02 §C, R04 §3.

## D12 Licence holder and repo visibility
- **Decision:** LICENSE copyright holder **"Vin's Atelier"** (owner instruction; overrides the root template's legal-name form). Repo becomes **public at first push**, only after the pre-public gate (`check-identity.sh --public`, `_house/pre-public/CHECKLIST.md`, `security-review`) passes. No visibility change after that.
- **Research:** root `CLAUDE.md` §8 (override recorded here).

## D13 Credits placement
- **Decision:** an editor has no page bottom, so the house credits card + draggable logo live in the **Welcome screen footer** and the **About dialog**. About also credits FFmpeg (LGPLv2.1 notice, links to ffmpeg.org, the licence and our source archive) and the logo artist (Portus Workstation, @portus_workstation / @wason25_pz).
- **Research:** root `CLAUDE.md` §2, R04 §7 item 6.

## D14 Keymaps and onboarding
- **Decision:** default "Editelier" keymap plus optional Premiere-style and Resolve-style keymaps (key-to-action conventions only, no assets). Welcome asks "What are you making?" (YouTube / Short / Reel / Photo / Carousel → canvas + export preset) and an optional "coming from" (sets keymap). Every action also reachable from a menu/button; `?` opens a searchable cheat sheet.
- **Research:** R03 §5–6.

## D15 Test media
- **Decision:** small self-generated clips (ffmpeg `testsrc2`/`smptehdbars` + `sine`, every tricky variant) are committed. External media (Blender open movies CC BY, raw.pixls.us CC0 checked per file, Xiph derf per clip) is fetched by script into gitignored `test-media/` via a manifest with URL, SHA-256, licence, attribution. **Never Pexels.** Owner-shot phone clips (no people) for real-device cases.
- **Research:** R05 §B7.

## D16 CI
- **Decision:** GitHub Actions on `windows-latest` (public repo), but root rules still apply: verify locally, batch pushes, don't watch CI. Add the workflow at **M1 end**, not now.
- **Research:** root `CLAUDE.md` §7.

## D17 Subagents on Opus (owner override)
- **Decision:** all subagents for this project run on Opus (overrides root §1's lighter-model default). The orchestrator still dispatches; builders write code.

## D18 Landing page on Cloudflare Pages (owner override)
- **Decision:** small Astro site in `site/`, built after M1 is usable, deployed to **Cloudflare Pages** per owner (root default is Workers static assets). Downloads link to GitHub Releases; hosts the privacy policy (`docs/PRIVACY.md`).

## D19 Clean-room and naming
- **Decision:** no proprietary assets, layouts, icons or trade dress. Never use Premiere, DaVinci, Resolve or CapCut in the product name, icon, Store listing or marketing; factual comparisons in docs are fine, and the optional keymap names use them descriptively only. GPL/AGPL projects are study-only (no code, shaders or text copied).
- **Research:** R03 clean-room note, R02 §B.

## D20 Colour is the product's centre (owner, 2026-11-26)
- **Goal:** anyone can easily make vibrant, cinematic colour-graded video **and** photos (one-tap looks with depth for tweaking), like films and cinematic YouTubers. Colour moves from M4 to M2 (`docs/PLAN.md`).
- **Pipeline (R06 §4.1):** decode → tagged Y'CbCr→RGB → input transform → scene-linear f16 → **per-clip grade before the D5 compositor** (titles and graphics are never graded; project-level grain may sit over the composite) → display rendering transform → Rec.709 → compositor → encode. Grade stages run in a **fixed, hidden order: Fix → Look → Film**.
- **Spaces:** blending, blur, exposure, halation and bloom in **scene-linear**; contrast, wheels, curves and look tone shaping in an **ACEScct-style log** grading space (ACES is Apache-2.0); hue, saturation, vibrance and skin work in **OKLCh**.
- **Own invertible tone curve (DRT):** a clean-room parametric sigmoid; SDR sources go through its exact inverse (capped at about +3 stops), so a **neutral grade is identity on SDR** (A-COL-01). Log, HDR and RAW sources skip the inverse. AgX, Filmic and darktable are **ideas only** (AgX/Filmic repos carry no licence file; darktable is GPL).
- **Looks:** **strength slider 0–150 %** (parametric looks scale parameters toward identity; LUT looks mix in log space). **Skin protect on by default** (soft OKLCh hue qualifier, attenuates hue and saturation pushes only). Built-in looks are our own clean-room parameter sets, shipped as MIT JSON, with our own names: no stock, film-stock or brand names, **no third-party LUTs** or HaldCLUTs.
- **`.cube` import (M5):** untrusted input, validated strictly (size 2–256, exact entry count, finite floats, file ≤ 64 MB); tetrahedral interpolation in WGSL.
- **Photos** use the identical pipeline and looks.
- **Why:** the owner's end goal is graded edits; one-tap looks differentiate a free editor where multitrack does not; where the grade sits is a pipeline decision that is cheap now and expensive to retrofit after more golden frames exist.
- **Research:** R06 §4, §5, §6 and summary items 8–11; R03 (looks-first UX); R05 §A3.

## D21 Photo scope: shared pipeline, not a second app (owner, 2026-11-26)
Photos are one-frame clips through the same grade pipeline (D20): import, Looks + grade, light/colour, crop/straighten, text, copy grade to many, carousel export, photos on the timeline. Not planned: spot healing/retouch, masks/layers, a photo library. Reason: the colour goal transfers to stills almost for free and carousel + reel from one project is a differentiator; a full photo editor would dilute video and colour quality. RAW develop stays (M9) because RAW gives the most grading room. Plan: `docs/PLAN.md` M4.

## D22 Editelier has its own logo (owner, 2026-11-26)
- **Decision:** every project has its own logo; the Vin's Atelier studio logo appears only in the credits sign-off (Welcome footer + About credits, D13). Editelier's mark ("tracks": a playhead beside three coloured clips that reads as an E) and its Space Grotesk wordmark live in `brand/` and drive the favicon, top bars, About header, Tauri icon and MSIX tiles. The mark is a placeholder-quality original by the dev team, all rights reserved (`NOTICE.md`); the owner may commission a replacement, which only needs new SVGs in `brand/` and the regenerate steps in `brand/README.md`.
- **Research:** root `CLAUDE.md` §2 Brand; `brand/README.md` (options explored, legibility sheet).

## D23 H.264/HEVC probe details come from Media Foundation (2026-11-27)
- **Finding:** the shipped LGPL ffprobe (D3, `packaging/ffmpeg-build.md`) has no H.264/HEVC/AAC decoders or parsers, so it reports `pix_fmt`, `profile` and bitstream (VUI) colour as `unknown`; container fields (size, fps, duration, rotation, sample rate/channels, `colr` atom colour) still come through.
- **Decision:** `probe` fills the missing bit depth (`pix_fmt`), primaries, transfer, matrix, range and HDR of the first H.264/HEVC video stream from Media Foundation's native media type (`IMFSourceReader::GetNativeMediaType`: `MF_MT_VIDEO_PROFILE`, `MF_MT_VIDEO_PRIMARIES`, `MF_MT_TRANSFER_FUNCTION`, `MF_MT_YUV_MATRIX`, `MF_MT_VIDEO_NOMINAL_RANGE`). No decode, so no HEVC extension is needed; ffprobe values win where present, anything MF can't tell stays unknown. Windows-only (`win::mf::fill_native`). AAC needs nothing (rate/channels come from the container). Debug builds now prefer `third_party/ffmpeg-lgpl/bin` over the dev build so dev matches release.
- **Why:** keeps patented-codec bitstream parsing out of our FFmpeg while 10-bit / HLG / PQ / BT.2020 detection still works.
- **Research:** `packaging/ffmpeg-build.md` (Configuration); Microsoft Learn, Media Type Attributes.

## D24 M2 Looks contract: one grade model shared by core, renderer and UI (2026-11-28)
- **Model (`editelier-core`, platform-free, serde):** `Clip.grade: Grade` (`#[serde(default)]`, so v1 projects load neutral). `Grade { fix: Fix, look: Option<String> /* look id */, strength: f32 /* 0..=1.5, default 1.0 */, film: Option<Film> /* None = the look's film */, skin_protect: f32 /* 0..=1, default 0.7 */ }`. `Fix { exposure /* EV −3..3 */, temp, tint /* −1..1 */ }`. Project look: `Timeline.project_grade: Option<Grade>`, used by clips whose `grade.look` is `None` and whose `fix` is neutral.
- **Look library:** the 12 built-in looks of R06 §6.2 as MIT JSON in `crates/editelier-core/looks/*.json` (embedded with `include_str!`), each `{ id, name, blurb, version, params: LookParams, film: Film }`. `LookParams` follows R06 §6.1 (exposure, temp, tint, contrast, pivot, toe, shoulder, black_lift [r,g,b], white_cap, sat, vibrance, split {sh_hue, sh_amt, hi_hue, hi_amt, balance}, bands[8] {hue_shift, sat, lum}, mix: Option<[f32;3]> for mono). `Film { halation, halation_radius, bloom, grain, grain_size, vignette, matte: Option<f32> /* aspect */ }`.
- **Strength (core, pure fn):** `LookParams::at(strength)` lerps every parameter from identity, extrapolating past 1.0 to 1.5 and clamping each to its valid range. The renderer never sees strength, only the resolved params (`ResolvedGrade { fix, look: LookParams, film: Film, skin_protect }` from `Grade::resolve(&library)`).
- **Commands (undoable, JSON like the rest):** `set_grade { clips: [Id], grade, parts: "all"|"fix"|"look"|"film" }` (copy/paste and multi-select), `set_project_grade { grade: Option<Grade> }`, `apply_grade_to_all { from: Id }`. Strength drags coalesce with `drag` like trims.
- **Renderer (`editelier-render`):** the WGSL `grade()` stage becomes one fused pass driven by a uniform built from `ResolvedGrade`: Fix (exposure, WB via CAT02 in linear) → Look (ACEScct log tone ops: contrast/pivot/toe/shoulder; OKLCh sat/vibrance/bands/split; skin protect mask) → DRT → black lift/white cap (output code values) ; film: halation + bloom (downsampled pyramid, linear, before DRT), grain (seeded by frame index, luminance-weighted, last) and vignette/matte. `Grade` neutral must stay bit-identical to today (A-COL-01). HLG/PQ sources: inverse OETF/EOTF into scene-linear, no SDR inverse-DRT. Bypass flag for before/after (whole frame or split at x).
- **UI:** right-side Looks panel: strip of 12 live thumbnails (192×108 renders of the current frame, `grade_thumbnails`), strength slider 0–150 %, skin-protect chip with "show protected area", Fix sliders (Light, Warmth, Tint), scope (This clip / Selected / Whole project), Apply to all, Copy/Paste grade (Ctrl+Alt+C/V), hold `\` for before, draggable split. Looks first, knobs behind "More".
- **Why:** one source of truth (core) keeps the UI, agent channel, project files and renderer in step, and strength-as-parameter-scaling (R06 §4.2) works the same on photos (D21).
- **Research:** R06 §4.1–4.3, §6; D20.
