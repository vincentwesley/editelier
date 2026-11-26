"""Writes Editelier's logo SVGs: hand-authored geometry + "Editelier" in Space Grotesk (OFL) outlined via fontTools.
Usage: python brand/make-logo.py   (needs fonttools + brotli; reads the house woff2 font)."""
from pathlib import Path
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

ROOT = Path(__file__).resolve().parent
FONT = ROOT.parent / "ui/src/house/fonts/space-grotesk-latin-wght-normal.woff2"
DARK = dict(spine="#e9f0df", a="#4fe0a0", b="#e9a07f", c="#a996e0", text="#e9f0df")
LIGHT = dict(spine="#10251a", a="#14673a", b="#b4532a", c="#6b55b8", text="#10251a")
MONO = dict(spine="currentColor", a="currentColor", b="currentColor", c="currentColor", text="currentColor")


def rects(items):
    return "".join(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{f}"/>' for x, y, w, h, f in items)


# Chosen mark ("tracks"): a playhead beside three clips on three tracks; together they read as an E.
def tracks(p):
    return rects([(6, 6, 8, 52, p["spine"]), (18, 6, 40, 14, p["a"]), (18, 25, 28, 14, p["b"]), (18, 44, 40, 14, p["c"])])


# Explore B ("channels"): three colour channels knocked out of register.
def channels(p):
    return rects([(22, 22, 36, 36, p["c"]), (14, 14, 36, 36, p["b"]), (6, 6, 36, 36, p["a"])])


# Explore C ("chart"): crop marks around a three-band colour chart.
def chart(p):
    s, k = p["spine"], 4
    marks = [(4, 4, 14, k, s), (4, 4, k, 14, s), (46, 4, 14, k, s), (56, 4, k, 14, s),
             (4, 56, 14, k, s), (4, 46, k, 14, s), (46, 56, 14, k, s), (56, 46, k, 14, s)]
    return rects(marks + [(14, 14, 36, 12, p["a"]), (14, 26, 36, 12, p["b"]), (14, 38, 36, 12, p["c"])])


def svg(view, body, label="Editelier"):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}" role="img" aria-label="{label}">{body}</svg>\n'


def wordmark(text="Editelier", size=50, x0=76, baseline=52):
    font = TTFont(FONT)
    instantiateVariableFont(font, {"wght": 600}, inplace=True)
    gs, cmap, hmtx = font.getGlyphSet(), font.getBestCmap(), font["hmtx"]
    scale = size / font["head"].unitsPerEm
    pen, x = SVGPathPen(gs), 0.0
    for ch in text:
        name = cmap[ord(ch)]
        gs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, x0 + x * scale, baseline)))
        x += hmtx[name][0] - 10  # slight tightening, like the -0.01em heading tracking
    return pen.getCommands(), round(x0 + x * scale) + 4


def main():
    (ROOT / "explore").mkdir(exist_ok=True)
    for name, fn in (("a-tracks", tracks), ("b-channels", channels), ("c-chart", chart)):
        (ROOT / "explore" / f"{name}.svg").write_text(svg("0 0 64 64", fn(DARK)), encoding="utf-8")
    d, width = wordmark()
    for name, pal in (("dark", DARK), ("light", LIGHT), ("mono", MONO)):
        (ROOT / f"mark-{name}.svg").write_text(svg("0 0 64 64", tracks(pal)), encoding="utf-8")
        body = tracks(pal) + f'<path fill="{pal["text"]}" d="{d}"/>'
        (ROOT / f"wordmark-{name}.svg").write_text(svg(f"0 0 {width} 64", body), encoding="utf-8")
    # App icon / favicon source: the dark mark on the house background with clear space.
    tile = f'<rect width="64" height="64" fill="#11150e"/><g transform="translate(8 8) scale(0.75)">{tracks(DARK)}</g>'
    (ROOT / "icon-tile.svg").write_text(svg("0 0 64 64", tile), encoding="utf-8")
    # Copies the UI serves (Vite root is ui/, so it can't import from here).
    ui = ROOT.parent / "ui"
    (ui / "src/brand").mkdir(exist_ok=True)
    for f in ("mark-dark", "mark-light", "wordmark-dark", "wordmark-light"):
        (ui / "src/brand" / f"{f}.svg").write_text((ROOT / f"{f}.svg").read_text(encoding="utf-8"), encoding="utf-8")
    (ui / "public/favicon.svg").write_text(svg("0 0 64 64", tile), encoding="utf-8")


if __name__ == "__main__":
    main()
