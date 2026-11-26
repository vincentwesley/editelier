# Research index

All research dated 2026-11-25 (06: 2026-11-26), written by research agents into `docs/research/` (don't hand-edit). Every claim there carries its own source link. Decisions built on it: `docs/DECISIONS.md`.

| # | File | Topic |
|---|---|---|
| 01 | [research/01-filmcraft.md](research/01-filmcraft.md) | FilmCraft deep dive (storytold/filmcraft @ `2d1d1b6`) |
| 02 | [research/02-oss-editors.md](research/02-oss-editors.md) | Other OSS editors + reusable Rust crates |
| 03 | [research/03-ux.md](research/03-ux.md) | UX: Premiere Pro, DaVinci Resolve, CapCut, photo editors |
| 04 | [research/04-windows-media-legal.md](research/04-windows-media-legal.md) | Windows preview surface, decode/encode, patents, MSIX, AI licences, name check, FFmpeg LGPL |
| 05 | [research/05-standards-tooling.md](research/05-standards-tooling.md) | Timing/colour/audio standards, test approach, Claude Code tooling |
| 06 | [research/06-colour-grading.md](research/06-colour-grading.md) | Colour grading: cinematic one-tap looks for video and photos, pipeline, legal |

## Summaries

**01 FilmCraft.** Pure-Rust clean-room NLE targeting Premiere parity, `MIT OR Apache-2.0` (ArtCraft marks excluded), alpha v0.20.1, very fast-moving and agent-built. UI is forked egui on wgpu (its rules ban webviews/Tauri); Windows is its weakest platform (MF decode read back to CPU, NVENC-only HW encode, no MSIX). Verdict: don't fork; reuse `audio-dsp`, `color`, `scopes` and the 254,016,000,000 ticks/s time type under MIT; copy the command registry, copy-on-write undo, ffmpeg-oracle, golden-frame and GPU-parity test designs. Spike the wgpu-under-WebView2 preview first, egui/wgpu pane as fallback.

**02 OSS editors.** OpenReel and OpenCut are MIT (reusable with attribution); MediaBunny is MPL-2.0 (unmodified dependency only); LosslessCut is GPL-2.0 and Kdenlive/Shotcut/Palmier/OpenTake/OpenChatCut are GPL/AGPL (study only). OpenCut classic's MIT wgpu crates (`compositor`/`effects`/`masks`/`time`) are the best compositor seed to evaluate. Crate picks: num-rational, ebur128, cosmic-text + glyphon, cpal, rubato, whisper-rs, ort, image, rsmpeg. RAW/HEIC libraries are LGPL: prefer Windows WIC, else DLL or helper process.

**03 UX.** Users praise Premiere's flexibility, Resolve's free power, CapCut's ease and one-click captions. Top pains to beat: laggy 4K/HEVC on laptops, export paywalls, crashes and lost work, VFR phone-footage drift, steep learning curve, export-setting confusion. Proposals: magnetic main track + free tracks, J/K/L, I/O, Q/W, S split, destination-first export cards, "What are you making?" onboarding, journaled autosave, plain-language errors, optional Premiere-style/Resolve-style keymaps (conventions only), looks-first photo editing and a carousel mode.

**04 Windows media + legal.** Preview: wgpu (DX12) in a child HWND beside the WebView2, overlays drawn in wgpu (canvas streaming costs ~250 MB/s at 1080p30). Patents: H.264/HEVC/AAC pools license whoever ships the codec, so route them through Windows Media Foundation and ship FFmpeg without its software h264/hevc/aac; AV1 + Opus are royalty-free. tauri-bundler has no MSIX: use Microsoft `winapp` CLI, `runFullTrust`, no `broadFileSystemAccess`, Win11 min. AI: whisper.cpp/Whisper/distil-whisper MIT, MODNet/MediaPipe Apache, BiRefNet MIT (verify), RMBG excluded. "Editelier": no conflicts in USPTO/Store/GitHub/registries; IPOS/WIPO/EUIPO need manual checks; editelier.com is taken.

**05 Standards + tooling.** Integer/rational time, keyframe seek + decode-forward, honour edit lists on import and write none on export. VFR preserved on timeline (sample-and-hold), conformed at export. Linear-light Rec.709 f16 working space, tag-driven 601/709 and range, BT.709-tagged exports. 48 kHz mix, `ebur128`, −14 LUFS/−1 dBTP social and −23 broadcast. Tests: ffmpeg oracle (framemd5, PTS, tags), golden frames ≤2/255 + SSIM ≥0.995, VMAF ≥93 at milestones; E2E via WebdriverIO tauri service with debug frame hashes. Own JSON-RPC named-pipe automation channel, debug-only and locked down.

**06 Colour grading.** The owner's end goal (vibrant, cinematic graded video and photos in one tap). Measured reference frames share lifted teal-leaning blacks, low overall saturation with 1–2 hero colours, warm subject / cool environment, highlight roll-off, halation and grain. UX lesson: fixed hidden order Fix → Look → Film, few well-named looks, strength to 150 %, apply-to-all, paste with re-masking. Pipeline: per-clip grade before the compositor in scene-linear with ACEScct-style log for tone ops and OKLCh for hue/sat/skin, own invertible DRT so SDR round-trips unchanged, skin protect on by default, strict `.cube` import with tetrahedral WGSL. Iris Xe estimate 3–5 ms/frame at 1080p. Legal: ship only our own parameter looks (MIT); AgX/Filmic (no licence) and darktable (GPL) as ideas only; no third-party LUTs or film-stock names. Proposes 12 built-in looks and moving colour to M2 (adopted: D20).

## Tools: installed / listed

**Installed:** none yet.

**Listed (evaluate when the shell exists, dev-only, pin versions):**

| Tool | Use | Source |
|---|---|---|
| Tauri MCP: P3GLEG/tauri-plugin-mcp (preferred, token auth, refuses release builds) or hypothesi/mcp-server-tauri (pick one) | Claude sees and drives the dev app | [P3GLEG](https://github.com/P3GLEG/tauri-plugin-mcp), [hypothesi](https://github.com/hypothesi/mcp-server-tauri) |
| WebdriverIO + `@wdio/tauri-service` (+ `@axe-core/webdriverio`) | E2E + a11y on WebView2, auto msedgedriver | [WDIO Tauri](https://webdriver.io/docs/desktop-testing/tauri/platform-support), [WDIO axe](https://webdriver.io/docs/accessibility-testing/axe-core) |
| Tracy (`tracing-tracy`, `wgpu-profiler`) | CPU/GPU profiling behind a feature | [tracing-tracy](https://docs.rs/crate/tracing-tracy/0.11.0), [wgpu-profiler](https://docs.rs/crate/wgpu-profiler/0.15.0) |
| PresentMon | Frame-pacing / dropped-frame measurement | [GameTechDev/PresentMon](https://github.com/GameTechDev/PresentMon) |
| libvmaf (via the local GPL ffmpeg, test oracle only, never shipped) | VMAF/SSIM/PSNR quality gates | [Netflix/vmaf](https://github.com/Netflix/vmaf), [filter docs](https://ayosec.github.io/ffmpeg-filters-docs/5.0/Filters/Video/libvmaf.html) |

Skipped: FFmpeg MCP servers, Windows-MCP, WDIO MCP, community Tauri skills (reasons in 05 §C).
