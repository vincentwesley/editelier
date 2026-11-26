# Third-party notices

Paths below are relative to `ui/src/house/`.

Bundled fonts (`fonts/`), all under the SIL Open Font License 1.1, full text in `fonts/OFL.txt`:
- Space Grotesk
- Inter
- Silkscreen
- JetBrains Mono

Pixel kit and icons (`pixel/`, `icons/`):

| Asset | Author | Source | Licence | Files |
|---|---|---|---|---|
| 1-Bit Pack (tiles cut from the monochrome sheet) | Kenney | https://kenney.nl/assets/1-bit-pack | CC0 1.0 | `pixel/icons.png`, `pixel/deco.png`, `pixel/404.png` |
| Pixel UI Pack (`9-Slice/Ancient/grey.png` reduced to an outline ring) | Kenney | https://kenney.nl/assets/pixel-ui-pack | CC0 1.0 | `pixel/frame.png` |
| Cursor Pixel Pack 1.0 (tiles 26 and 134 of `tilemap_packed.png`) | Kenney | https://kenney.nl/assets/cursor-pixel-pack | CC0 1.0 | `pixel/cursors/pointer.png`, `pixel/cursors/hand.png`, `pixel/cursors/LICENSE.txt` |
| Coins, Gems, etc. 2.0 (five sprites cut from the sheet) | GreatDocBrown | https://greatdocbrown.itch.io/coins-gems-etc | CC0 1.0 | `pixel/loot.png` |
| Pixelarticons 2.4.2 | Gerrit Halfmann | https://github.com/halfmage/pixelarticons | MIT (`icons/LICENSE`) | `icons/*.svg` (`alert.svg` is `warning-diamond`) |
| ZzFX Micro 1.4.0 (ES module, lazy AudioContext) | Frank Force | https://github.com/KilledByAPixel/ZzFX | MIT | `js/zzfx.js` |


Vendored Rust source (`crates/vendor/`), each with its licence file and a `PROVENANCE` note:

| Crate | Author | Source | Licence | Files |
|---|---|---|---|---|
| craft-time (Tick, TimeRange, FrameRate subset) | ArtCraft Team and the craft-libs contributors | https://github.com/storytold/craft-libs/tree/a4db799563c9ccd7217e1731ec34c58f6ba57248/crates/time | MIT (dual MIT OR Apache-2.0 upstream; taken under MIT, `LICENSE-MIT`) | `crates/vendor/craft-time/src/lib.rs` |

Commissioned artwork and the credits character are **all rights reserved, not MIT**: see `NOTICE.md`.

## Rust crates (`crates/editelier-media`)

Compiled into the app, all permissive: `windows`/`windows-core` 0.62 (MIT OR Apache-2.0), `serde_json` (MIT OR Apache-2.0), `rubato` 5.1 + `audioadapter*`, `rustfft`, `num-*`, `transpose`, `strength_reduce`, `primal-check` (MIT OR Apache-2.0), `realfft`, `windowfunctions`, `zmij` (MIT), `audio-codec-algorithms` (0BSD OR Apache-2.0), `memchr` (Unlicense OR MIT), `unicode-ident` (MIT OR Apache-2.0 AND Unicode-3.0). Full list: `cargo tree -p editelier-media -e normal -f "{p} {l}"`.

## Title fonts (`crates/editelier-render/fonts/`)

Static TTF instances (converted from the house kit's WOFF2 variable fonts with fontTools: woff2 → ttf, `wght` pinned) of the same OFL 1.1 fonts, compiled into the app for title rendering; full licence text in `crates/editelier-render/fonts/OFL.txt`. No Reserved Font Names are declared.
- `SpaceGrotesk-Bold.ttf`: Space Grotesk, wght 700, Copyright 2020 The Space Grotesk Project Authors
- `Inter-SemiBold.ttf`: Inter, wght 600, Copyright (c) 2016 The Inter Project Authors
- `JetBrainsMono-Bold.ttf`: JetBrains Mono, wght 700, Copyright 2020 The JetBrains Mono Project Authors

## Rust crates (`crates/editelier-render`)

All permissive: `wgpu` 30 and its `wgpu-*`/`naga` crates (MIT OR Apache-2.0), `cosmic-text` 0.19, `fontdb`, `swash`, `skrifa`/`read-fonts`, `harfrust`, `unicode-*` (MIT OR Apache-2.0; `unicode-linebreak` Apache-2.0), `slotmap`, `foldhash` (Zlib), `libloading` (ISC), `codespan-reporting` (Apache-2.0), `self_cell` (Apache-2.0 OR GPL-2.0-only; taken under Apache-2.0). Full list: `cargo tree -p editelier-render -e normal -f "{p} {l}"`.

## Rust crates (`src-tauri`, the app)

Beyond the crates above and Tauri's own (listed in `docs/spikes/preview-surface.md`): `cpal` 0.18 (Apache-2.0) with `dasp_*` (MIT OR Apache-2.0) for audio output, `tauri-plugin-dialog` 2.8 (Apache-2.0 OR MIT) with `rfd` (MIT) for file pickers, `ebur128` (MIT, via `editelier-export`). Full list: `cargo tree -p editelier -e normal -f "{p} {l}"`.

## FFmpeg (not in the repo; bundled in the MSIX)

Dev builds run `ffprobe.exe`/`ffmpeg.exe` from a BtbN `lgpl-shared` build that `scripts/fetch-ffmpeg.ps1` fetches into gitignored `third_party/ffmpeg/` (LGPL; this BtbN variant is built with `--enable-version3`, so LGPL-3.0-or-later). `cargo xtask package` copies `ffmpeg.exe`, `ffprobe.exe` and the `av*`/`sw*` DLLs (original names) into the MSIX, with `licences/FFmpeg/LICENSE.txt` and `licences/FFmpeg/BUILD-FFMPEG.txt` (configure line, pin, source links: https://github.com/FFmpeg/FFmpeg, https://github.com/BtbN/FFmpeg-Builds). Shipped builds will use our own LGPL build with the software H.264/HEVC/AAC codecs disabled (D3, `packaging/ffmpeg-build.md`); the libraries bundled inside it get listed here when it lands.

## MSIX package (`packaging/`)

Visual assets in `packaging/Assets/` are generated from Editelier's own mark (`brand/`, D22), all rights reserved (see `NOTICE.md`), not MIT. The package also carries `LICENSE`, `NOTICE.md`, this file and the fonts' `OFL.txt` under `licences/`.
