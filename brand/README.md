# Editelier logo

Editelier's own mark (D22). All rights reserved (see `NOTICE.md`), not MIT. The Vin's Atelier studio logo is a separate thing and only appears in the credits sign-off (D13).

The mark is a playhead beside three clips on three tracks; together they read as an E. The clips carry the three house accents (mint, peach, lavender) as a nod to colour grading. Square corners, flat fills, no gradients, so it holds at 16px.

## Files
| file | use |
|---|---|
| `mark-dark.svg` | mark on dark backgrounds (default) |
| `mark-light.svg` | mark on light backgrounds (`data-theme="light"`) |
| `mark-mono.svg` | one colour, `currentColor` |
| `wordmark-dark.svg` / `wordmark-light.svg` / `wordmark-mono.svg` | mark + "Editelier" (Space Grotesk 600, outlined) |
| `icon-tile.svg` | mark on a `#11150e` tile: favicon and app-icon source |
| `png/icon-1024.png` | `cargo tauri icon` source (`src-tauri/icons/icon.ico`) |
| `png/mark-dark-1024.png` | `packaging/make-assets.ps1` source (MSIX tiles) |
| `png/sheet.png` | legibility check: every option at 16/32/64/256 px, light mark, wordmarks |
| `explore/` | the three options considered (a-tracks chosen, b-channels, c-chart) |

## Regenerate
1. `python brand/make-logo.py` (fonttools + brotli): writes the SVGs, the UI copies in `ui/src/brand/` and `ui/public/favicon.svg`.
2. `node brand/render.mjs` (Playwright from `ui/node_modules`): writes `png/`.
3. `cargo tauri icon brand/png/icon-1024.png -o <temp dir>`, copy `icon.ico` to `src-tauri/icons/`.
4. `powershell -File packaging/make-assets.ps1`: MSIX tiles.

## Rules
- Dark variant by default; light only under `data-theme="light"` (house `.hs-brand-dark` / `.hs-brand-light` swap).
- Don't recolour, stretch, reorder the bars or add effects. Minimum size 16px for the mark, 24px tall for the wordmark.
- Clear space: at least one bar height (14/64 of the mark) on every side, except app-icon tiles, where the built-in margin is enough.
- Never combine it with the studio `</>` glyph or use it in the studio's place.

## Why this option
"Tracks" says what the app does (a timeline: playhead and clips) and spells its initial without any lettering, its colour bars tie to grading (D20), and its flat rectangles survive 16px taskbar size where the other two blur: the offset squares of "channels" turn to mush below 32px, and the crop marks of "chart" vanish at 16px.
