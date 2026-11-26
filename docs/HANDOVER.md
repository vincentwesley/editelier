# Handover

Updated at each stopping point. Current: **M1 feature-complete, not yet tagged**, `main`, 2026-11-26.

## Done + verified
- M0: research `docs/research/01–06`, decisions D1–D22, plan, acceptance, legal review, privacy, needs-Vin. Public repo `vincentwesley/editelier` (pre-public gate + security review passed).
- Preview surface: wgpu DX12 child HWND above WebView2, 60 fps on Iris Xe (`docs/spikes/preview-surface.md`).
- `editelier-core`: tick time (vendored craft-time, MIT), magnetic main track + title + music tracks, undoable commands, project JSON v1 (tests green).
- `editelier-media`: probe (VFR, rotation, colour tags), PTS/keyframe index, frame-exact MF decode (DXVA), thumbnails, waveforms, proxies, EXIF (8 orientations), A/V start alignment, streamed audio cache (tests green).
- `editelier-export`: MF encoders (QSV found; AMF/NVENC paths ready), presets, ebur128 + true-peak limiter; VMAF ≈ 99.9 on synthetic pattern; conformant MP4 (ffprobe).
- `editelier-render`: YUV→linear Rec.709, identity grade stage (neutral round trip ≤ 1/255), titles (house fonts), orientation, NV12 output.
- App: Tauri commands per the UI contract, playback, export job; zero-copy decode→render→encode.
- Perf (Iris Xe, under load): export 1080p ≈ 1.9–2.2× realtime, 4K ≈ 1.0×; preview 1080p 30/30 fps, 4K proxy ≈ 40 fps, 4K original ≈ 38 fps bench.
- UI: welcome ("What are you making?"), editor, canvas timeline, keymaps (default/Premiere-style/Resolve-style), export dialog, About, toasts; Vitest 31, Playwright 18 + axe both themes.
- Real-app e2e (off-screen WebView2, agent channel): 8/8 flows incl. export verified by ffprobe.
- MSIX: `cargo xtask package --dev-cert` builds a signed 76 MB package (dev only; bundles the dev LGPLv3 FFmpeg).
- Security: agent channel reviewed clean; UNC/WebDAV project-path leak fixed; ffmpeg restricted to file protocol.

## Not done + why
- Own LGPLv2.1 FFmpeg build: needs Docker/WSL2/MSYS2 toolchain; until then the MSIX must not be published (A-LEGAL-01 fails by design).
- MSIX install/launch/uninstall checks: need the dev cert trusted (owner admin step).
- UI-driven export in e2e: native folder picker can't be automated yet.
- Usability walkthroughs, design review, full acceptance run, CI, v0.1.0 tag: next session (see `docs/next-session.md`).

## Needs Vin
See `docs/NEEDS-VIN.md`.
