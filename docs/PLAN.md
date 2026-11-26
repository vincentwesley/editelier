# Plan

Milestones in user-value order. Each ships something a creator can use end to end. Exit = every listed ACCEPTANCE id green (`cargo xtask accept --milestone mN`, see `docs/ACCEPTANCE.md`) plus the milestone tier from root `CLAUDE.md` §4. Decisions referenced as D-n (`docs/DECISIONS.md`).

Test hardware: i5-1335U / Iris Xe / 16 GB / 1920×1200 laptop now; AMD desktop from Jan 2027 (AMF path).

## Crate layout (planned, created in M1)

```
Cargo.toml              workspace
src-tauri/              Tauri 2 shell: window, child-HWND viewer, invoke → command registry
crates/
  core/                 platform-free: tick time, project model, command registry, CoW undo, project JSON
  media/                platform-free Media trait + frame/stream types
  media-win/            Media Foundation + D3D11: H.264/HEVC/AAC decode/encode, shared-handle import (D3)
  media-ffmpeg/         LGPL ffprobe/ffmpeg helper processes: probe, demux/mux, PTS/keyframe index, royalty-free codecs (D3 amendment)
  render/               wgpu: per-clip grade (Fix → Look → Film) + DRT, linear-light f16 compositor, titles (cosmic-text + glyphon) (D5, D20)
  audio/                48 kHz mix, cpal output clock, loudness (D6)
  proxy/                proxy, thumbnail and waveform-peak cache (D4)
  agent/                `agent` feature JSON-RPC (lives in src-tauri/src/agent.rs) named pipe, debug + feature-gated (D7)
  vendor/               filmcraft-audio-dsp, filmcraft-color, filmcraft-scopes, craft-time (D2)
xtask/                  check, accept, layers, fetch-media, ffmpeg-build, package (MSIX)
ui/                     TS + Vite UI, house kit
site/                   landing page (after M1)
test-media/             fetched corpus (gitignored) + committed synthetic fixtures
```

Layering rule (checked by `cargo xtask layers`): `core`/`media` depend on no OS, UI or GPU crate; only `media-win`, `src-tauri` and `agent` touch Windows APIs.

## M1: Cut, title, music, export, install

Scope, in order:
1. **Spike:** wgpu DX12 child HWND under a reserved WebView2 region (D1); proves resize/DPI/modal handling and zero-copy D3D11→wgpu. Fail → egui/wgpu pane fallback, recorded in DECISIONS.
2. Import: probe, thumbnails, waveforms, VFR flag (D4); drag from Explorer or Import button.
3. One main track: trim, split, join, ripple delete; snapping; J/K/L, I/O, Q/W, S.
4. Undo/redo for every edit (command registry + CoW snapshots).
5. Auto proxies in the background with a status indicator (D4).
6. Title clip: text, house fonts (OFL), cosmic-text + glyphon.
7. Music track: volume, fade in/out.
8. Identity grade stage: per-clip Grade node (Fix → Look → Film, all neutral) and the invertible DRT in the render graph before the compositor, with the neutral SDR round-trip golden test (D20).
9. Export: 1080p / 4K MP4 H.264 + AAC via MF hardware encoder (QSV → AMF → NVENC) with MF software fallback; YouTube / TikTok / Instagram presets as data files with `source_url` + `verified_on` (D3, D6).
10. Welcome ("What are you making?", "coming from") + About (credits card, draggable logo, FFmpeg notice) (D13, D14).
11. MSIX via winapp, dev cert (D9). Own LGPL FFmpeg build (D3).

Exit: A-CUT-01..07, A-QUAL-01..07 (06: social preset), A-PERF-01..07, A-UI-01..06, A-IMPORT-01..03, A-MSIX-01..05, A-LEGAL-01..05, A-COL-01. Then: CI workflow (D16), pre-public gate, first push (public, D12), tag `v0.1.0`, GitHub Release (unsigned MSIX + test-cert steps + portable ZIP + FFmpeg source archive).

**Landing page (after M1):** Astro `site/` on Cloudflare Pages (D18): download links, screenshots, privacy policy, credits.

## M2: Looks
Scope (D20, R06 §6): Fix (exposure, white balance, auto); 12 built-in looks from R06 §6.2 (our names: Sunlit Film, Quiet Street, Blue Hour Room, Warm Close-up, Clean Pop, Golden Hour, Teal & Ember, Faded Memory, Print Classic, Neon Night, Silver, Natural) with live thumbnails; strength 0–150 %; film layer (halation, bloom, grain, vignette, optional letterbox crop); skin protect (on by default, "show protected area"); before/after (hold `\`, draggable split); apply to all / project look; copy-paste grade; HLG→SDR via the DRT and Apple Log input.
Exit: A-COL-01..05, plus M1 ids still green.

## M3: Multi-track editing
Scope: multiple video/audio tracks with magnetic main track + free tracks; audio mixer with R128 meters and loudness-normalised export (both presets); transitions (dissolve, dip, wipe); keyframes with easing; speed change (rubato for audio); journaled autosave + crash reopen; AV1 + Opus export.
Exit: A-CUT-08..09, A-QUAL-06 (broadcast preset too), A-UI-07, plus earlier ids still green.

## M4: Photos and mixed projects
Scope (owner, 2026-11-26, D21): photos share the video pipeline; no separate photo app. In: import stills (JPEG/PNG/HEIC via WIC, D11), the M2 Looks + grade panel, light/colour sliders, crop/straighten, text, before/after, copy grade to many; photos as timeline clips with slow pan/zoom; mixed photo + clip projects; carousel mode (4:5 slides, seamless panorama split, numbered 1080-wide JPEG/PNG export) and reel from the same project. Out (not planned): spot healing/retouch, masks/layers, photo library/catalogue. RAW stays M9 because it serves the colour goal.
Exit: A-IMPORT-04 (stills), A-QUAL-08 (carousel slice exactness), A-UI-08, A-COL-02 on photo fixtures.

## M5: Pro colour
Scope: colour wheels (lift/gamma/gain/offset), curves, HSL curves, split tone, `.cube` LUT import (strict validation, tetrahedral WGSL; D20), scopes (waveform, parade, histogram, vectorscope from vendored `filmcraft-scopes`), match to a reference frame, shot matching across clips; user looks saved/exported as `.editelier-look`.
Exit: A-COL-06, A-QUAL-09 (LUT + grade golden frames), A-QUAL-10 (scope parity vs CPU reference).

## M6: Auto captions
Scope: one-button on-device captions (whisper.cpp, D10), transcript editor, low-confidence highlights, style presets, burn-in or SRT/VTT export.
Exit: A-QUAL-11 (WER on a CC-licensed speech fixture), A-PERF-08 (captioning speed), A-LEGAL-06.

## M7: Background removal
Scope: person/background matte for video (MODNet or MediaPipe Selfie) and stills (BiRefNet-lite if licence verified), editable edges, via ort + DirectML (D10). The matte can also restrict skin protect spatially (R06 §4.2).
Exit: A-QUAL-12 (matte IoU on fixture), A-PERF-09, A-LEGAL-06.

## M8: User shader effects
Scope: user-written WGSL effects with a parameter schema (sliders generated from it), sandboxed compile with clear errors, saved with the project.
Exit: A-QUAL-13 (shader golden frames), A-UI-09 (bad shader shows an error, never crashes).

## M9: RAW develop
Scope: camera RAW import via WIC first, LGPL DLL/helper fallback (D11); white balance, exposure, highlights, demosaic options; RAW enters the grade pipeline as scene-linear (D20).
Exit: A-IMPORT-05 (RAW corpus), A-QUAL-14.

## M10: "How it works" pipeline view
Scope: optional view walking one frame from sensor RAW → demosaic → colour → encode → compressed frame, using the app's real pipeline stages.
Exit: A-UI-10.

## M11: Interchange and switcher polish
Scope: OTIO (own serde model, Apache-2.0 schema), FCPXML, CMX 3600 EDL import/export; keymap and layout polish for Premiere/Resolve switchers.
Exit: A-CUT-10 (round-trip edit decisions frame-exact), A-UI-11.

IDs beyond M1 are placeholders; define their thresholds in `docs/ACCEPTANCE.md` when the milestone starts.
