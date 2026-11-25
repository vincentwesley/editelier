# 01 — FilmCraft (storytold/filmcraft)

Research date: 2026-11-25. Snapshot: `main` @ [`2d1d1b6`](https://github.com/storytold/filmcraft/commit/2d1d1b6920cf14c166e1b2bc73ba8208def150c9) (workspace version 0.20.1). Read-only shallow clone; nothing was built or run.

Link shorthand: every `[x](…)` below points at that commit unless it is an issue, release or external page.

## 0. What it is

- A pure-Rust, clean-room professional NLE that explicitly targets **Adobe Premiere Pro 2026 parity** ("Target: Adobe Premiere Pro 2026 (26.5.2)") ([architecture.md L3](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L3), [ROADMAP.md](https://github.com/storytold/filmcraft/blob/2d1d1b6/ROADMAP.md)).
- Licence: `MIT OR Apache-2.0`; the ArtCraft name/logos are trademarks, excluded, and forks must remove them ([README L464-L481](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L464-L481), [NOTICE](https://github.com/storytold/filmcraft/blob/2d1d1b6/NOTICE)). GitHub reports Apache-2.0 ([repo](https://github.com/storytold/filmcraft)).
- Repo created 2026-09-30; 19k stars, 7.6k forks, ~1,418 open issues as of today ([repo](https://github.com/storytold/filmcraft), [open issues](https://github.com/storytold/filmcraft/issues)).
- Developed largely by AI agents: the roadmap budgets work in "Opus 5.5 agent-hours" and the repo ships a strict agent rulebook ([ROADMAP.md headline table](https://github.com/storytold/filmcraft/blob/2d1d1b6/ROADMAP.md), [AGENTS.md](https://github.com/storytold/filmcraft/blob/2d1d1b6/AGENTS.md)). `CLAUDE.md` is just `@AGENTS.md` ([CLAUDE.md](https://github.com/storytold/filmcraft/blob/2d1d1b6/CLAUDE.md)).
- Sister app **PhotoCraft** (layers, masks, PSD) is a separate repo, relevant to our photo side ([README L421](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L421), [photocraft](https://github.com/storytold/photocraft)).

## 1. Architecture and crate map

Cargo workspace of ~33 crates plus 3 apps and `xtask`, with **enforced layering** (`cargo xtask layers`: only downward edges, no UI/OS crates below L5) ([architecture.md §1](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L22-L94), [Cargo.toml](https://github.com/storytold/filmcraft/blob/2d1d1b6/Cargo.toml)).

```
L6  apps/filmcraft (desktop) · filmcraft-cli · filmcraft-web (wasm)
L5  ui-egui · automation (MCP) · platform (OS media FFI, the only unsafe crate)
L4  engine (Session, command registry, undo, jobs)
L3  render (CPU reference compositor) · gpu (wgpu) · export · scopes · golden (tests)
L2  edit · codecs · interchange · captions · speech · tts · tts-text
L1  frame · media · project · audio-dsp · text
L0  time · geom · color · bitstream · isobmff · matroska · cfb · dng · testkit
shared (git deps from storytold/craft-libs): craft-video, craft-audio, craft-media, craft-timeline, craft-ui ...
```

Design principles: engine-first, everything is a command, exact integer time, copy-on-write `Arc<Project>` snapshots for undo, CPU reference + GPU fast path ([architecture.md L9-L20](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L9-L20)).

Notable facts (line counts from the clone):

| Crate | ~LOC | External deps | Notes |
|---|---|---|---|
| `ui-egui` | 104k | egui, eframe, craft-ui/host/window/theme | the whole UI ([architecture.md L70](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L70)) |
| `engine` | 72k | most workspace crates | `Session::execute(id, params)` ([§4](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L156-L193)) |
| `render` | 27k | rayon, serde | CPU reference compositor, effects, transitions, audio mix |
| `codecs` | 18k | craft-video/audio/media, symphonia | demux hub, GOP-aware seek, decoder registry |
| `platform` | 17k | none (raw FFI) | VideoToolbox / Media Foundation / VA-API / NVENC ([ADR 0001](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/adr/0001-platform-ffi.md)) |
| `interchange` | 16k | craft-timeline, craft-aaf, roxmltree | EDL, FCP7 XML, FCPXML, OTIO, AAF, OMF ([README](https://github.com/storytold/filmcraft/blob/2d1d1b6/crates/interchange/README.md)) |
| `audio-dsp` | 15k | **none** | BS.1770/R128 loudness, EQ, dynamics, limiter, 5.1, sync ([architecture.md L56](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L56)) |
| `project` | 12k | serde, url | document model, effect definitions as data |
| `gpu` | 6.6k | wgpu | WGSL compositor + effects |
| `color` | 2.7k | serde only | spaces, transfer functions, YUV↔RGB, LUTs ([architecture.md L42](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L42)) |
| `scopes` | 1.3k | color, serde | waveform, parade, histogram, vectorscope |
| `time` | 19 | craft-timeline | re-export of `craft_timeline::time` ([architecture.md L40](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L40)) |

**Important:** the codecs (H.264/HEVC/VP9/AV1/ProRes/AAC...), containers and the time type no longer live in this repo; they are pinned git dependencies on [storytold/craft-libs](https://github.com/storytold/craft-libs) (`MIT OR Apache-2.0`) ([Cargo.toml L84-L92](https://github.com/storytold/filmcraft/blob/2d1d1b6/Cargo.toml#L84-L92), [architecture.md L49-L51](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L49-L51)). It also patches egui/eframe/winit to storytold forks ([Cargo.toml L156-L171](https://github.com/storytold/filmcraft/blob/2d1d1b6/Cargo.toml#L156-L171)).

## 2. UI toolkit and how preview frames reach the screen

- **egui 0.37 / eframe on wgpu** (forked), not Tauri/iced ([Cargo.toml L106-L113](https://github.com/storytold/filmcraft/blob/2d1d1b6/Cargo.toml#L106-L113)). Their suite rules forbid webviews: "Apps SHOULD be Rust only: no handwritten JavaScript or TypeScript, Tauri, Electron or webview" ([AGENTS.md L26](https://github.com/storytold/filmcraft/blob/2d1d1b6/AGENTS.md?plain=1#L26)). So an Editelier Tauri front end could never go upstream.
- Preview path: worker pool in `ui-egui/src/frames.rs` renders (on-screen frame first, then prefetch, then thumbnails; the UI never decodes) ([architecture.md L345-L365](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L345-L365)). The GPU compositor shares eframe's wgpu device and the composited texture is handed to egui with `register_native_texture` / `update_egui_texture_from_wgpu_texture` (zero copy from compositor to UI) ([ui-egui/src/app/gpu.rs L44-L76](https://github.com/storytold/filmcraft/blob/2d1d1b6/crates/ui-egui/src/app/gpu.rs#L44-L76)). GPU errors fall back to the CPU compositor ([gpu.rs L5-L30](https://github.com/storytold/filmcraft/blob/2d1d1b6/crates/ui-egui/src/app/gpu.rs#L5-L30)).
- Decode → GPU is **not** zero-copy: Media Foundation's D3D11 NV12/P010 texture is read back to CPU and re-uploaded ([architecture.md L261-L271](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L261-L271), [hardware-parity.md L35](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/hardware-parity.md?plain=1#L35)).
- Audio clock: cpal output drives the playhead; video follows ([architecture.md L366-L371](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L366-L371)).
- Windows UI complaints: flicker, low frame rate, tab rendering, non-native menu bar ([#5112](https://github.com/storytold/filmcraft/issues/5112), [#4485](https://github.com/storytold/filmcraft/issues/4485), [#4359](https://github.com/storytold/filmcraft/issues/4359), [#5071](https://github.com/storytold/filmcraft/issues/5071)).

## 3. wgpu compositor, colour pipeline, effects/shaders

- **Compositor** (`crates/gpu`, WGSL: `composite.wgsl`, `fx.wgsl`, `lut.wgsl`, `mask.wgsl`, `transition.wgsl`) samples YUV from the decoder with footprint supersampling and blends in linear light; backends Metal/Vulkan/DX12/WebGPU ([README L148](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L148), [crates/gpu/src](https://github.com/storytold/filmcraft/tree/2d1d1b6/crates/gpu/src)).
- **The CPU renderer is the reference.** `render::plan::plan_frame` turns a frame into GPU layers; all 27 blend modes and ~34 common effects run on GPU as compute passes "with the CPU reference's math"; anything else (most grading, keys, masks, non-dissolve transitions, nested sequences) is rendered on CPU and uploaded as a layer image ([architecture.md L301-L329](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L301-L329)). **Export renders on the CPU** for grading/keys ([README L303](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L303)).
- **Colour:** working space is linear light, premultiplied `f32`, "1.0 = reference white = SDR white = 203 cd/m²"; working space Rec.709 or Rec.2100 PQ/HLG; BT.2390 tone map; monitors are SDR BT.709 (sRGB RGBA8); HDR export to BT.2020 PQ/HLG with `mdcv`/`clli`/SEI ([architecture.md §5.4 L553-L609](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L553-L609)). Rec.709-only media/sequences take the GPU fast path; log/HDR/wide-gamut layers are converted on CPU (same lines). Untagged media defaults to BT.709 with "assumed" reported. LUTs: `.cube`/`.3dl` with a WGSL tetrahedral `GpuLut` (same section).
- **Effect API:** effect definitions are data — `project::effect::effect_defs()` lists every effect with a parameter schema that generates the Effects panel, Effect Controls rows and agent docs ([architecture.md L150-L152](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L150-L152)). Effects are compiled-in Rust (CPU) plus fixed WGSL (GPU).
- **User shaders: no.** No plugin hosting at all: no OpenFX, VST3/AU; listed as gap G11, "owner decision", 200-400 h ([gaps.md L222](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/gaps.md?plain=1#L222-L228), [README L305](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L305)).

## 4. Timing model

- All time is `Tick(i64)` at **254,016,000,000 ticks/s**, which divides every broadcast frame duration (23.976 … 120) and every common sample rate (8-192 kHz, incl. 44.1k family) ([architecture.md §2 L96-L113](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L96-L113)). `FrameRate { num, den }` rational; `TimeRange` half-open; timecode (NDF/DF) is display-only; commands accept ticks/frame/seconds/timecode, converted once at the boundary (same). Code lives in craft-libs `craft-timeline` ([architecture.md L40](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L40)).
- Timeline positions are sequence ticks; `source_in` and keyframes are in media time, so trims never move keyframes ([architecture.md L148-L149](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L148-L149)).
- **VFR:** a nominal rate is derived per track (mean of sample durations near the median, snapped to a standard rate within 0.5 %) ([codecs/src/mp4.rs L34-L57](https://github.com/storytold/filmcraft/blob/2d1d1b6/crates/codecs/src/mp4.rs#L34-L57)); near-CFR phone files with ms-rounded timestamps are snapped back to the frame grid, but a file with any picture more than ¼ frame off keeps its real times ("variable frame rate") and the shown frame is "the last one that starts at or before" the requested time ([mp4.rs L806-L819](https://github.com/storytold/filmcraft/blob/2d1d1b6/crates/codecs/src/mp4.rs#L806-L819)). No conform-to-CFR pass. A real-world VFR corpus is listed as missing ([gaps.md G6 L124-L139](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/gaps.md?plain=1#L124-L139)).
- **Seeking:** seek to preceding sync sample, decode forward, cache the whole GOP; late non-reference frames are skipped during catch-up; draft decoding optional ([architecture.md L237-L240, L345-L363](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L237-L240)). MXF/TS oracle tests check "random seeks = sequential decode", bit-exact ([testing.md L95-L175](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/testing.md?plain=1#L95-L175)).

## 5. Media I/O

- **No FFmpeg at all, not even LGPL.** Codecs/containers are their own clean-room Rust (H.264 enc/dec, HEVC/VP8/VP9/AV1/MPEG-2 decode, ProRes and DNxHR enc/dec, AAC enc/dec, Opus...); ffmpeg is used only as an external test oracle and never linked or shipped ([README L175-L197](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L175-L197), [testing.md L95-L99](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/testing.md?plain=1#L95-L99)). Contributors MUST NOT read GPL/LGPL/AGPL source ([AGENTS.md L41](https://github.com/storytold/filmcraft/blob/2d1d1b6/AGENTS.md?plain=1#L41)). MP3/FLAC/ALAC/Vorbis decode still via symphonia (MPL-2.0) ([README L191](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L191)).
- **Windows Media Foundation, yes — decode only:** a D3D11-aware decoder MFT driven per access unit (no Source Reader), DXVA via `IMFDXGIDeviceManager`, H.264 / HEVC Main/Main10 / VP9 / AV1, falls back to their software decoder (never Windows' software MFT); readback to CPU ([architecture.md L261-L274](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L261-L274)). FFI is hand-written in the `platform` crate ([ADR 0001](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/adr/0001-platform-ffi.md)).
- **HW encode on Windows: NVENC only** (H.264 opt-in, H.265 hardware-only incl. Main10 HDR); **no QSV, no AMF**; no AV1 export; H.265 export is impossible without an NVIDIA GPU on Windows ([hardware-parity.md L37-L46](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/hardware-parity.md?plain=1#L37-L46), [architecture.md L275-L294](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L275-L294), [gaps.md G5](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/gaps.md?plain=1#L107-L122), [#4293](https://github.com/storytold/filmcraft/issues/4293)). NVENC is loaded at runtime (`nvEncodeAPI64.dll`) from a transcribed MIT header ([ATTRIBUTION.md §Third-party source code](https://github.com/storytold/filmcraft/blob/2d1d1b6/ATTRIBUTION.md?plain=1#L169-L176)).
- Self-admitted risk: "camera and phone files in the wild are less tested. Windows and Linux get far less testing than macOS" ([README L307](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L307)); real bugs like iPhone HEVC export failure, WebM black, only first audio track ([gaps.md L133-L136](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/gaps.md?plain=1#L133-L136)).

## 6. Interchange

FCP7 XML, FCPXML 1.9-1.11, OpenTimelineIO (round-trips via `metadata.filmcraft`), CMX 3600 EDL, AAF Edit Protocol, OMF 2.0, plus Avid ALE; no `.prproj` ([README L216-L227](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L216-L227), [interchange README](https://github.com/storytold/filmcraft/blob/2d1d1b6/crates/interchange/README.md)). OTIO and XML are implemented natively in Rust (craft-timeline features `xml`, `otio`), not via the C++ OTIO library ([interchange/Cargo.toml L13](https://github.com/storytold/filmcraft/blob/2d1d1b6/crates/interchange/Cargo.toml#L13)). The crate is coupled to FilmCraft's `project` model, so it is not drop-in for another data model. Never validated in Avid/Pro Tools ([gaps.md G15](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/gaps.md?plain=1#L294-L300)).

## 7. Agent / MCP control channel

- Every action is a `CommandSpec` (id, label, menu path, shortcut, param doc string, `enabled()` with a reason, `run()`, `journal` flag); 650+ engine commands; UI, CLI, control channel and MCP dispatch the same ids ([architecture.md L156-L193](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L156-L193), [README L231-L244](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L231-L244)). UI widgets register automation ids, so agents can click/drag/type/screenshot ([architecture.md §7](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/architecture.md?plain=1#L733-L760)).
- Control channel: JSON-lines over TCP bound to `127.0.0.1`, requires a 64-hex `FILMCRAFT_CONTROL_TOKEN` per request, rejects non-JSON/HTTP lines (anti-CSRF from browsers), 4 MiB line cap, 16 connections max ([control-protocol.md L3-L41](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/control-protocol.md?plain=1#L3-L41)).
- MCP: `filmcraft-cli mcp` on stdio via `rmcp` (Apache-2.0), headless in-process session or `--bridge` to the running app; tools `command_list`, `command_run`, `command_batch`, `render_frame`, `ui_click`, `ui_screenshot`… ([control-protocol.md L215-L224](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/control-protocol.md?plain=1#L215-L224), [.mcp.json](https://github.com/storytold/filmcraft/blob/2d1d1b6/.mcp.json)).
- **Gap:** their own rule says MCP entry points MUST refuse to start without read/write root confinement ([AGENTS.md L33](https://github.com/storytold/filmcraft/blob/2d1d1b6/AGENTS.md?plain=1#L33)), yet the docs state "there are no folder-confinement flags … the server can read and write wherever the user can" ([agents.md L17-L19](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/agents.md?plain=1#L17-L19)). Lesson for us: build path confinement in from day one.

## 8. Testing

- **ffmpeg as oracle**: external process, discovered by `testkit`; skipped locally if missing, mandatory in CI (`FILMCRAFT_REQUIRE_ORACLES=1`); fixtures synthetic (`testsrc2`, bars, mandelbrot) generated into `target/fixtures`, never committed ([testing.md L95-L130](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/testing.md?plain=1#L95-L130)). Per-codec criteria: H.264/HEVC bit-exact every frame; ProRes ±1 LSB; DNx ±2; containers compared packet-by-packet with `ffprobe -show_packets`; loudness ±0.1 LU vs `ebur128`; export checked by ffprobe + own decoder ([testing.md L147-L175](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/testing.md?plain=1#L147-L175)).
- **Golden frames + GPU parity**: CPU renders of procedural 320×180 scenes vs committed PNGs (PSNR ≥ 45 dB, max abs ≤ 12, p99 ≤ 2); GPU vs CPU p99 ≤ 6 / mean < 1.5 (8-bit levels); failure writes actual + ×8 diff; transitions checked by fingerprints and endpoint identities ([testing.md §3 L176-L226](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/testing.md?plain=1#L176-L226)). GPU parity already caught a real stale-texture cache bug (same section).
- Other: proptest for time/edit/round-trips, seeded fuzz/truncation ("nothing may panic"), headless scripted UI via `egui_kittest` driven over the control channel ([testing.md §1](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/testing.md?plain=1#L20-L52)).
- **CI**: shared `craft-common-actions` gate (`@main`, not SHA-pinned) on Linux with Mesa lavapipe and `FILMCRAFT_REQUIRE_GPU=1`, plus `ci-windows.yml` and `windows-arm64.yml` ([ci.yml](https://github.com/storytold/filmcraft/blob/2d1d1b6/.github/workflows/ci.yml), [ci-windows.yml](https://github.com/storytold/filmcraft/blob/2d1d1b6/.github/workflows/ci-windows.yml), [testing.md L417-L424](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/testing.md?plain=1#L417-L424)). Media Foundation and NVENC tests skip without the hardware/Store codec extensions ([testing.md L125-L145](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/testing.md?plain=1#L125-L145)).
- **Windows status**: "routine runtime testing off macOS" is missing; start-up crashes are gap G4 (wgpu/driver crashes, a DX12 → WARP fallback ladder exists) ([gaps.md G4 L82-L105](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/gaps.md?plain=1#L82-L105), [G9 L172-L180](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/gaps.md?plain=1#L172-L180), [README L344-L364](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L344-L364)). Open: export crash 0xc0000409 ([#4903](https://github.com/storytold/filmcraft/issues/4903)), update failure ([#4778](https://github.com/storytold/filmcraft/issues/4778)). 31 open issues carry the `windows` label ([label](https://github.com/storytold/filmcraft/issues?q=is%3Aissue+is%3Aopen+label%3Awindows)).

## 9. Packaging

- Windows: **MSI via WiX v5** (x64, arm64, x86) + portable ZIP, code-signed; per-user default under `Programs\ArtCraft\<App>` with per-machine option; nightly is a ZIP. **No MSIX** anywhere in `packaging/` ([README L334-L342](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L334-L342), [packaging/windows/filmcraft.wxs](https://github.com/storytold/filmcraft/blob/2d1d1b6/packaging/windows/filmcraft.wxs), [INSTALLATION.md](https://github.com/storytold/filmcraft/blob/2d1d1b6/packaging/windows/INSTALLATION.md), [package.ps1](https://github.com/storytold/filmcraft/blob/2d1d1b6/packaging/windows/package.ps1)).
- Also DMG (notarised), AppImage/Flatpak/deb/rpm/tar, FreeBSD, web ZIP ([README L366-L411](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L366-L411)). Self-update via `craft-update` ([Cargo.toml](https://github.com/storytold/filmcraft/blob/2d1d1b6/Cargo.toml)).

## 10. Licences of notable dependencies

| Dependency | Licence | Flag |
|---|---|---|
| FilmCraft itself | MIT OR Apache-2.0 ([LICENSE-MIT](https://github.com/storytold/filmcraft/blob/2d1d1b6/LICENSE-MIT), [LICENSE-APACHE](https://github.com/storytold/filmcraft/blob/2d1d1b6/LICENSE-APACHE)) | ArtCraft marks excluded ([LICENSE-brand](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/brand/LICENSE-brand.txt)) |
| storytold/craft-libs (codecs, timeline, ui) | MIT OR Apache-2.0 ([Cargo.toml](https://github.com/storytold/craft-libs/blob/main/Cargo.toml)) | git-pinned, not on crates.io |
| egui/eframe fork, winit fork | Apache-2.0 / MIT upstream ([storytold/egui](https://github.com/storytold/egui), [storytold/winit](https://github.com/storytold/winit)) | forks |
| wgpu | MIT OR Apache-2.0 ([gfx-rs/wgpu](https://github.com/gfx-rs/wgpu)) | ok |
| cpal | Apache-2.0 ([RustAudio/cpal](https://github.com/RustAudio/cpal)) | ok |
| rmcp (MCP) | Apache-2.0 ([rust-sdk Cargo.toml](https://github.com/modelcontextprotocol/rust-sdk/blob/main/Cargo.toml)) | ok |
| candle (Whisper/Kokoro inference) | MIT OR Apache-2.0 ([huggingface/candle](https://github.com/huggingface/candle)) | ok |
| harfrust (shaping) | MIT ([harfbuzz/harfrust](https://github.com/harfbuzz/harfrust)) | ok |
| **symphonia** (MP3/FLAC/ALAC/Vorbis) | **MPL-2.0** ([Symphonia](https://github.com/pdeljanov/Symphonia)) | weak file-level copyleft; ship source pointer if bundled ([craft-contrib licences](https://github.com/storytold/craft-contrib/blob/main/standards/licences.md)) |
| egui default fonts | Ubuntu Font Licence 1.0, OFL-1.1, MIT, Bitstream Vera ([ATTRIBUTION.md §egui fonts](https://github.com/storytold/filmcraft/blob/2d1d1b6/ATTRIBUTION.md?plain=1#L189-L196)) | notices required |
| NVENC header transcription | MIT (NVIDIA) ([ATTRIBUTION.md L169-L176](https://github.com/storytold/filmcraft/blob/2d1d1b6/ATTRIBUTION.md?plain=1#L169-L176)) | ok |
| Whisper weights (runtime download) | MIT ([ATTRIBUTION.md L178-L187](https://github.com/storytold/filmcraft/blob/2d1d1b6/ATTRIBUTION.md?plain=1#L178-L187)) | opt-in download |

**No GPL, AGPL or non-commercial** code found; README states no GPL/LGPL code ([README L476](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L476)), and the suite allowlist excludes them ([licences.md](https://github.com/storytold/craft-contrib/blob/main/standards/licences.md)). Patent note (not licence): their own H.264/HEVC encoders/decoders are royalty-bearing formats regardless of code licence; using OS codecs (MF) shifts that to Microsoft, which is one reason our MF-first plan stays sensible.

Apache/NOTICE: because FilmCraft is `MIT OR Apache-2.0`, we may take it under **MIT** and only keep the copyright + permission notice; if we take it under Apache-2.0 we must carry its [NOTICE](https://github.com/storytold/filmcraft/blob/2d1d1b6/NOTICE) and mark changed files. Either way, bundled fonts/assets keep their own notices.

## 11. Maturity

- **Velocity:** 23 releases from v0.1.0 (2026-10-02) to v0.20.1 (2026-11-12), roughly every 2-3 days ([releases](https://github.com/storytold/filmcraft/releases)); 211 contributors, top author ~1,258 commits ([contributors](https://github.com/storytold/filmcraft/graphs/contributors)); 45 crates, ~343k lines Rust, 2,520 tests ([ROADMAP.md](https://github.com/storytold/filmcraft/blob/2d1d1b6/ROADMAP.md)).
- **Stage: alpha.** Feature breadth ~86 % of Premiere, "ready for real work" ~56 %, "essentials user" ~60 %, with Windows launch crashes and discoverability named as the main blockers for essentials users ([ROADMAP.md readiness table](https://github.com/storytold/filmcraft/blob/2d1d1b6/ROADMAP.md), [README L292-L310](https://github.com/storytold/filmcraft/blob/2d1d1b6/README.md?plain=1#L292-L310)).
- Works: editing/trim/multicam, grading, keyframes, 90+ effects, titles/captions, mixer, R128 meters, exports, interchange (README sections above). Weak: Windows runtime testing, real-world media, HW accel (no zero-copy, NVENC only), plugins, CPU-bound grading/export ([gaps.md](https://github.com/storytold/filmcraft/blob/2d1d1b6/docs/gaps.md)).
- Churn risk: codecs/time recently moved out to craft-libs; egui is forked and pinned; CI calls shared workflows at `@main` ([Cargo.toml](https://github.com/storytold/filmcraft/blob/2d1d1b6/Cargo.toml), [ci.yml](https://github.com/storytold/filmcraft/blob/2d1d1b6/.github/workflows/ci.yml)).

## Verdict: (b) reuse a few crates as dependencies, otherwise (c) study and build our own

**Not (a) fork.** Reasons:
1. **Wrong product shape.** It is a Premiere clone (docked panels, three-point editing, 650 commands); our goal is CapCut-level ease. Forking means un-building most of its 104k-line egui UI.
2. **Wrong UI stack for us, and upstream won't take ours.** egui immediate-mode vs our Tauri + TS UI; suite rules ban Tauri/webviews ([AGENTS.md L26](https://github.com/storytold/filmcraft/blob/2d1d1b6/AGENTS.md?plain=1#L26)), so a fork diverges permanently.
3. **Windows is their weakest platform** (G4/G9, flicker/crash issues, NVENC-only encode, no MSIX), while it is our first platform.
4. **Maintenance risk:** agent-driven, ~1.4k open issues, releases every 2-3 days, forked egui/winit, git-pinned craft-libs; tracking that as a fork would cost more than it saves.

**Reuse (b), via crates.io-free git pins or vendored with attribution under the MIT option:**
- `filmcraft-audio-dsp` — zero dependencies; R128/BS.1770 loudness verified against ffmpeg to ±0.1 LU, EQ/limiter/dynamics, 5.1 up/downmix, audio sync. Highest value, lowest coupling.
- `craft-timeline` time module (via craft-libs; what `filmcraft-time` re-exports) — or simply copy its 254,016,000,000 ticks/s `Tick` + rational `FrameRate` design (a few hundred lines; re-implement if the git dependency is too heavy).
- `filmcraft-color` (serde only) and `filmcraft-scopes` (color + serde) — transfer functions, YUV matrices, BT.2390 tone map, LUT parsing, waveform/vectorscope maths. Good fit for our Rust core and our photo side.
- Optional, study first: `craft-libs` EDL/FCPXML/OTIO parsers (inside `craft-timeline` with `xml`/`otio` features) if they can be used without FilmCraft's `project` model; `filmcraft-interchange` itself is too coupled.
- Do **not** reuse their codecs in place of Media Foundation + LGPL FFmpeg for now: real-world media coverage is their own self-declared weakness, and H.264/HEVC patent exposure is easier through OS codecs.

Licence hygiene for reuse: take under MIT, keep the `Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors` notice in vendored file headers, add rows to our `THIRD_PARTY_NOTICES.md` with repo URL + commit `2d1d1b6`, never ship ArtCraft/FilmCraft names or the owl icon, and avoid pulling `symphonia` (MPL-2.0) unless we accept the source-pointer duty.

**Study and copy the ideas (c):**
- Command registry as the single entry point (id, param schema, `enabled()` with reason, undo journalling) → maps perfectly onto Tauri `invoke` + an MCP server; start with it.
- `Arc<Project>` copy-on-write snapshots for undo and lock-free background readers.
- CPU reference renderer + GPU parity tests with explicit tolerances; ffmpeg as an external oracle with synthetic fixtures; golden frames with diff output.
- Layering enforced by an `xtask` (no UI/OS crates below the engine).
- Control channel hardening (loopback + per-session token + reject HTTP lines) and, unlike them, MCP file-root confinement from day one.
- Their VFR approach (nominal rate + grid snapping, keep true VFR times) and GOP cache with catch-up skipping.

**Should it change our planned stack?** Mostly no; two adjustments:
1. Keep Tauri 2 + Rust core + wgpu + MF/LGPL FFmpeg + MSIX. FilmCraft's success with wgpu in linear light confirms the compositor choice; its gaps (MF readback, no QSV/AMF) are things our MF-first, FFmpeg-for-breadth plan already addresses.
2. **De-risk the preview path early.** FilmCraft gets zero-copy compositor→screen only because egui shares the wgpu device. A Tauri webview cannot sample a wgpu texture, so spike preview presentation first: a native child swapchain/`HWND` composited under or beside the WebView2 surface (or WebView2 composition hosting), not frames encoded and sent over IPC. If that spike fails, an egui/wgpu-native preview pane inside the Tauri window is the fallback FilmCraft proves works.
3. Adopt a tick timebase compatible with theirs (254,016,000,000/s) so interchange and any reused crates line up.

## 10-line summary

1. FilmCraft: pure-Rust clean-room Premiere-clone NLE, MIT OR Apache-2.0 (ArtCraft marks excluded), alpha v0.20.1, 23 releases in 7 weeks, 211 contributors, ~1.4k open issues.
2. UI is a forked egui/eframe on wgpu; compositor texture goes to egui zero-copy, but MF decode is read back to CPU; suite rules ban Tauri/webviews.
3. Compositor: linear-light premultiplied f32, Rec.709/Rec.2100 working spaces, BT.2390 tone map; CPU renderer is the reference, GPU (WGSL) covers blends + ~34 effects; no user shaders/plugins.
4. Time: integer ticks at 254,016,000,000/s + rational frame rates; VFR via nominal-rate grid snapping; GOP-cached frame-accurate seeking.
5. Media: own codecs, no FFmpeg (oracle only); Windows MF HW decode, NVENC-only HW encode (no QSV/AMF); interchange OTIO/FCPXML/FCP7/EDL/AAF/OMF.
6. Agents: 650+ commands shared by UI/CLI/JSON-TCP (token-gated)/MCP (rmcp); MCP lacks file-root confinement despite its own rule.
7. Testing is excellent: ffmpeg oracle (bit-exact), golden frames, GPU-vs-CPU parity tolerances, lavapipe CI; Windows is their least-tested platform; packaging is WiX MSI, no MSIX.
8. No GPL/AGPL/non-commercial deps; only symphonia is MPL-2.0; take code under MIT to avoid Apache NOTICE duties, keep copyright headers.
9. Verdict (b)+(c): don't fork; reuse `filmcraft-audio-dsp`, `filmcraft-color`, `filmcraft-scopes` and the craft-timeline tick time type; copy the command-registry, CoW-undo, oracle/golden/parity test designs.
10. Stack stays (Tauri+Rust+wgpu+MF/LGPL FFmpeg+MSIX); adopt their tick timebase and spike the wgpu-preview-under-WebView2 path first, with an egui/wgpu native pane as fallback.
