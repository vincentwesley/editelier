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


Commissioned artwork and the credits character are **all rights reserved, not MIT**: see `NOTICE.md`.

## Rust crates (`crates/editelier-media`)

Compiled into the app, all permissive: `windows`/`windows-core` 0.62 (MIT OR Apache-2.0), `serde_json` (MIT OR Apache-2.0), `rubato` 5.1 + `audioadapter*`, `rustfft`, `num-*`, `transpose`, `strength_reduce`, `primal-check` (MIT OR Apache-2.0), `realfft`, `windowfunctions`, `zmij` (MIT), `audio-codec-algorithms` (0BSD OR Apache-2.0), `memchr` (Unlicense OR MIT), `unicode-ident` (MIT OR Apache-2.0 AND Unicode-3.0). Full list: `cargo tree -p editelier-media -e normal -f "{p} {l}"`.

## FFmpeg (not in the repo yet)

Dev builds run `ffprobe.exe`/`ffmpeg.exe` from a BtbN `lgpl-shared` build that `scripts/fetch-ffmpeg.ps1` fetches into gitignored `third_party/ffmpeg/` (LGPL-2.1-or-later). Shipped builds will use our own LGPL build with the software H.264/HEVC/AAC codecs disabled (D3). Its notice, licence text and source link go here when it lands.
