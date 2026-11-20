# Generates the Broadside logo in the website's look, each in a dark and a light theme:
#   assets/logo-{dark,light}.svg     the mark: a patch of chart with a B laid out as a fleet, one ship hit
#   assets/banner-{dark,light}.svg   the mark with the name and a line under it, for the README
# Flat, in the site's colours (navy ink on chart paper; light and dark as on the site). The name is
# set in Saira Stencil One, the line in Newsreader, as on the site. All text is turned into outlines,
# so the SVGs look the same whatever fonts the viewer has.
# Needs fontTools (pip install fonttools); the fonts (SIL OFL) are fetched from Google Fonts into
# ~/.cache/broadside-fonts on first run.
#   python3 assets/make_logo.py [outdir]
import os
import sys
import urllib.request

out = sys.argv[1] if len(sys.argv) > 1 else "assets"

# --- text as outlines ---
FONTS = {
    "display": ("https://github.com/google/fonts/raw/main/ofl/sairastencilone/SairaStencilOne-Regular.ttf", None),
    "text": ("https://github.com/google/fonts/raw/main/ofl/newsreader/Newsreader%5Bopsz%2Cwght%5D.ttf", {"opsz": 24, "wght": 400}),
}
_fonts = {}

def font(kind):
    if kind not in _fonts:
        from fontTools.ttLib import TTFont
        url, loc = FONTS[kind]
        cache = os.path.expanduser("~/.cache/broadside-fonts")
        path = os.path.join(cache, urllib.request.unquote(os.path.basename(url)))
        if not os.path.exists(path):
            os.makedirs(cache, exist_ok=True)
            urllib.request.urlretrieve(url, path)
        f = TTFont(path)
        _fonts[kind] = (f, f.getGlyphSet(location=loc) if loc else f.getGlyphSet(), kerning(f))
    return _fonts[kind]

def kerning(f):
    """Pair adjustments from the font's GPOS kern feature (pair positioning, formats 1 and 2)."""
    pairs, classed = {}, []
    if "GPOS" not in f:
        return pairs, classed
    gpos = f["GPOS"].table
    for fr in gpos.FeatureList.FeatureRecord:
        if fr.FeatureTag != "kern":
            continue
        for li in fr.Feature.LookupListIndex:
            lookup = gpos.LookupList.Lookup[li]
            for st in lookup.SubTable:
                if lookup.LookupType == 9:
                    st = st.ExtSubTable
                if getattr(st, "LookupType", lookup.LookupType) != 2:
                    continue
                if st.Format == 1:
                    for first, ps in zip(st.Coverage.glyphs, st.PairSet):
                        for pv in ps.PairValueRecord:
                            pairs.setdefault((first, pv.SecondGlyph), getattr(pv.Value1, "XAdvance", 0) or 0)
                else:
                    classed.append(st)
    return pairs, classed

def kern(table, a, b):
    pairs, classed = table
    if (a, b) in pairs:
        return pairs[(a, b)]
    for st in classed:
        if a in st.Coverage.glyphs:
            v = st.Class1Record[st.ClassDef1.classDefs.get(a, 0)].Class2Record[st.ClassDef2.classDefs.get(b, 0)].Value1
            if v is not None and getattr(v, "XAdvance", 0):
                return v.XAdvance
    return 0

def outline(kind, text, size, x, y, fill):
    """Text as one SVG path with its baseline at y, starting at x; returns (path, width)."""
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    f, gs, table = font(kind)
    k = size / f["head"].unitsPerEm
    names = [f.getBestCmap()[ord(ch)] for ch in text]
    advances, pen_x = [], 0.0
    for i, g in enumerate(names):
        advances.append(pen_x)
        pen_x += gs[g].width * k
        if i + 1 < len(names):
            pen_x += kern(table, g, names[i + 1]) * k
    pen = SVGPathPen(gs, ntos=lambda v: f"{v:.2f}".rstrip("0").rstrip("."))
    for g, ax in zip(names, advances):
        gs[g].draw(TransformPen(pen, (k, 0, 0, -k, x + ax, y)))
    return f'<path fill="{fill}" d="{pen.getCommands()}"/>', pen_x

# --- the mark ---
# A patch of chart with a B laid out as a fleet, as (column, row, length, vertical) on a
# 4 x 7 pixel B:   #XX.   the spine (7) down the left; the three bars (2) beside it;
#                  #..X   the two bowls (2) on the right
#                  #..X
#                  #XX.   Each hull keeps thin seams between its cells, so the letter keeps its
#                  #..X   pixels (as a pixel font draws B) while each ship stays one piece.
#                  #..X   One hit, on the upper bowl.
#                  #XX.
SHIPS = [(0, 0, 7, True), (1, 0, 2, False), (1, 3, 2, False), (1, 6, 2, False), (3, 1, 2, True), (3, 4, 2, True)]
HIT = (3, 1)

S = 512                     # canvas
T = 16                      # tile inset
RX = 104                    # tile corner radius
N = 9                       # cells across the tile
CELL = (S - 2 * T) / N
OX, OY = (N - 4) / 2, (N - 7) / 2   # the B's top left, in cells: centred (the grid shifts with it)
INSET = CELL * .07          # hull inset from its cells' edges
HULL_RX = CELL * .2         # hull corner radius
SEAM = CELL * .05           # the line between a ship's cells
EDGE = 10                   # tile outline

# The site's colours: the tile is its chart (water, ruled, outlined in ink), the ships ink.
THEMES = {
    "dark": dict(sea="#132439", rule="#26394f", ink="#e4e8e1", hit="#ef5f4c"),
    "light": dict(sea="#e3e8e4", rule="#c4ccd0", ink="#13294b", hit="#c8372d"),
}

def mark(t):
    """The mark's elements on a 512x512 canvas (no <svg> wrapper, so it can be placed in a banner)."""
    o = []
    w = S - 2 * T
    X, Y = T + OX * CELL, T + OY * CELL          # the B's origin
    o.append(f'<defs><clipPath id="tile"><rect x="{T}" y="{T}" width="{w}" height="{w}" rx="{RX}"/></clipPath></defs>')
    o.append(f'<rect x="{T}" y="{T}" width="{w}" height="{w}" rx="{RX}" fill="{t["sea"]}"/>')
    xs = [X + k * CELL for k in range(-N, N) if T < X + k * CELL < S - T]
    ys = [Y + k * CELL for k in range(-N, N) if T < Y + k * CELL < S - T]
    lines = "".join(f"M{x:.2f} {T}V{S - T}" for x in xs) + "".join(f"M{T} {y:.2f}H{S - T}" for y in ys)
    o.append(f'<path d="{lines}" stroke="{t["rule"]}" stroke-width="3" clip-path="url(#tile)"/>')
    for c, r, n, v in SHIPS:
        x, y = X + c * CELL + INSET, Y + r * CELL + INSET
        wd, ht = (CELL if v else n * CELL) - 2 * INSET, (n * CELL if v else CELL) - 2 * INSET
        o.append(f'<rect x="{x:.2f}" y="{y:.2f}" width="{wd:.2f}" height="{ht:.2f}" rx="{HULL_RX:.2f}" fill="{t["ink"]}"/>')
        # Seams between the hull's cells: each ship stays one piece, but the B keeps its cells.
        for k in range(1, n):
            p = (Y + (r + k) * CELL) if v else (X + (c + k) * CELL)
            seam = f"M{x:.2f} {p:.2f}H{x + wd:.2f}" if v else f"M{p:.2f} {y:.2f}V{y + ht:.2f}"
            o.append(f'<path d="{seam}" stroke="{t["sea"]}" stroke-width="{SEAM:.2f}"/>')
    # The hit: a cross in the sea's colour cut into the hull, then red over it.
    cx, cy = X + (HIT[0] + .5) * CELL, Y + (HIT[1] + .5) * CELL
    a = CELL * .22
    cross = f"M{cx - a:.2f} {cy - a:.2f}L{cx + a:.2f} {cy + a:.2f}M{cx - a:.2f} {cy + a:.2f}L{cx + a:.2f} {cy - a:.2f}"
    o.append(f'<path d="{cross}" stroke="{t["sea"]}" stroke-width="{CELL * .3:.2f}" stroke-linecap="round"/>')
    o.append(f'<path d="{cross}" stroke="{t["hit"]}" stroke-width="{CELL * .15:.2f}" stroke-linecap="round"/>')
    o.append(f'<rect x="{T + EDGE / 2}" y="{T + EDGE / 2}" width="{w - EDGE}" height="{w - EDGE}" rx="{RX - EDGE / 2}" fill="none" stroke="{t["ink"]}" stroke-width="{EDGE}"/>')
    return "\n".join(o)

# --- the banner: the mark, the name beside it, a line under the name ---
BANNER = {
    "dark": dict(paper="#0e1b2c", edge="#26394f", name="#e4e8e1", line="#9aa9ba"),
    "light": dict(paper="#eff1ec", edge="#c4ccd0", name="#13294b", line="#51627a"),
}

def banner(t, mt):
    W, H, PAD = 1280, 400, 64
    ms = H - 2 * PAD                                   # the mark, square, flush with the padding
    tx = PAD + ms + 56                                 # where the text column starts
    _, w100 = outline("display", "Broadside", 100, 0, 0, "none")
    size = min(168, (W - PAD - tx) / w100 * 100)       # the name fits the column
    name, _ = outline("display", "Broadside", size, tx, 214, t["name"])
    line, _ = outline("text", "An exact Battleship engine", 46, tx + 6, 288, t["line"])
    return "\n".join([
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="Broadside, an exact Battleship engine">',
        "<title>Broadside</title>",
        f'<rect x=".75" y=".75" width="{W - 1.5}" height="{H - 1.5}" rx="27.25" fill="{t["paper"]}" stroke="{t["edge"]}" stroke-width="1.5"/>',
        f'<g transform="translate({PAD} {PAD}) scale({ms / S:.5f})">', mark(mt), "</g>",
        name, line,
        "</svg>"]) + "\n"

for name, theme in THEMES.items():
    open(f"{out}/logo-{name}.svg", "w").write(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}" width="{S}" height="{S}" role="img" aria-label="Broadside">\n'
        f"<title>Broadside</title>\n{mark(theme)}\n</svg>\n")
    open(f"{out}/banner-{name}.svg", "w").write(banner(BANNER[name], theme))
print("ok")
