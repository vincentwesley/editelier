# Icons

32 SVGs from Pixelarticons (MIT, `LICENSE` here), all `fill="currentColor"`; `alert.svg` is Pixelarticons `warning-diamond`.
Use as an image-mask so they theme: `.i-menu { --icon: url("icons/menu.svg"); }` then `<span class="hs-icon i-menu" aria-hidden="true"></span>` (`.hs-icon` is in `pixel/pixel.css`).
Inline `<svg>` also works (copy the file); an `<img>` ignores `currentColor`, so avoid it.
