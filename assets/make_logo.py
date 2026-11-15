# Generates the Broadside logo, each in a dark and a light theme:
#   assets/logo-{dark,light}.svg     the mark: a 10x10 Battleship chart with a B laid out as a fleet,
#                                     two hits, a reticle on the second, misses over open sea
#   assets/banner-{dark,light}.svg   the mark with the name beside it, for the README
# The name is set in Saira Stencil One (the browser game's display face), the line under it in
# Barlow, the chart's coordinates in JetBrains Mono. All text is turned into outlines, so the
# SVGs look the same whatever fonts the viewer has.
# Needs fontTools (pip install fonttools); the fonts (SIL OFL) are fetched from Google Fonts
# into ~/.cache/broadside-fonts on first run.
#   python3 assets/make_logo.py [outdir]
import os
import sys
import urllib.request

out = sys.argv[1] if len(sys.argv) > 1 else "assets"

# --- text as outlines ---
FONTS = {
    "display": ("https://github.com/google/fonts/raw/main/ofl/sairastencilone/SairaStencilOne-Regular.ttf", None),
    "body": ("https://github.com/google/fonts/raw/main/ofl/barlow/Barlow-SemiBold.ttf", None),
    "mono": ("https://github.com/google/fonts/raw/main/ofl/jetbrainsmono/JetBrainsMono%5Bwght%5D.ttf", {"wght": 700}),
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

def outline(kind, text, size, x, y, fill, tracking=0.0, anchor="start"):
    """Text as one SVG path with its baseline at y, starting at x (or centred on it); returns (path, width)."""
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    f, gs, table = font(kind)
    k = size / f["head"].unitsPerEm
    names = [f.getBestCmap()[ord(ch)] for ch in text]
    advances, pen_x = [], 0.0
    for i, g in enumerate(names):
        advances.append(pen_x)
        pen_x += gs[g].width * k + tracking
        if i + 1 < len(names):
            pen_x += kern(table, g, names[i + 1]) * k
    width = pen_x - tracking
    if anchor == "middle":
        x -= width / 2
    pen = SVGPathPen(gs, ntos=lambda v: f"{v:.2f}".rstrip("0").rstrip("."))
    for g, ax in zip(names, advances):
        gs[g].draw(TransformPen(pen, (k, 0, 0, -k, x + ax, y)))
    return f'<path fill="{fill}" d="{pen.getCommands()}"/>', width

# --- the mark ---
B = ["11110", "10001", "10001", "11110", "10001", "10001", "11110"]
LIT = {(2 + c, 1 + r) for r, line in enumerate(B) for c, bit in enumerate(line) if bit == "1"}
HITS = {(2, 4), (6, 2)}              # E3, C7
MISSES = [(9, 1), (1, 3), (8, 6), (0, 8), (4, 9)]
RETICLE = (6, 2)

S = 512                              # canvas
FRAME, RIM = 17, 7                   # outer tile inset, width of its bevelled rim
RX = 96
X0, CELL = 82.5, 37.1                # board origin and cell size
GAP, TRX = 2.2, 6                    # tile inset and corner radius

THEMES = {
    "dark": dict(
        rim=[(0, "#a6dcf8"), (.035, "#3f8fca"), (.12, "#0f2c48"), (.85, "#0b2640"), (.96, "#0a4d82"), (1, "#1a6aa6")],
        face=[(0, "#04182c"), (1, "#020e1d")],
        board=[(0, "#031629"), (.55, "#03182c"), (1, "#06497c")],
        outline="#5b8fbf", edge="#6aa4d0", edge_glow=.9, grid="#173b58", label="#8fb6da",
        hull=[(0, "#fdfdfe"), (.55, "#d2deea"), (1, "#a6bdd2")], shadow="#000a16", shadow_op=.7,
        miss="#7ec5f9", miss_glow=True, halo=.42),
    "light": dict(
        rim=[(0, "#fbfdff"), (.04, "#e3ebf4"), (.12, "#ccd9e7"), (.88, "#c6d5e5"), (.97, "#a3b8cd"), (1, "#c3d2e2")],
        face=[(0, "#dae6f4"), (1, "#e8f2fd")],
        board=[(0, "#d6e5f4"), (.6, "#e0edfa"), (1, "#f1f9ff")],
        outline="#b4c6d9", edge="#4a6d8e", edge_glow=0, grid="#c9daec", label="#1d3a55",
        hull=[(0, "#ffffff"), (.6, "#eef3f8"), (1, "#d9e3ee")], shadow="#5d7894", shadow_op=.45,
        miss="#456b8c", miss_glow=False, halo=.3),
}

def stops(lst):
    return "".join(f'<stop offset="{o}" stop-color="{c}"/>' for o, c in lst)

def mark(t):
    """The mark's elements on a 512x512 canvas (no <svg> wrapper, so it can be placed in a banner)."""
    b = CELL * 10
    o = []
    o.append(f'''<defs>
  <linearGradient id="rim" x1="0" y1="0" x2="0" y2="1">{stops(t["rim"])}</linearGradient>
  <linearGradient id="face" x1="0" y1="0" x2="0" y2="1">{stops(t["face"])}</linearGradient>
  <radialGradient id="board" cx="50%" cy="50%" r="72%">{stops(t["board"])}</radialGradient>
  <linearGradient id="hull" x1="0" y1="0" x2="0" y2="1">{stops(t["hull"])}</linearGradient>
  <radialGradient id="hit" cx="50%" cy="45%" r="62%">
    <stop offset="0" stop-color="#ffd3c2"/><stop offset=".22" stop-color="#ffab98"/><stop offset=".6" stop-color="#fd3f37"/><stop offset="1" stop-color="#e8120b"/>
  </radialGradient>
  <radialGradient id="halo" cx="50%" cy="50%" r="50%">
    <stop offset=".45" stop-color="#ffbe0a" stop-opacity="{t["halo"]}"/><stop offset=".8" stop-color="#ffbe0a" stop-opacity="{t["halo"] * .35:.3f}"/><stop offset="1" stop-color="#ffbe0a" stop-opacity="0"/>
  </radialGradient>
  <filter id="shadow" x="-20%" y="-20%" width="140%" height="150%">
    <feGaussianBlur in="SourceAlpha" stdDeviation="1.6"/><feOffset dy="2.2" result="s"/>
    <feFlood flood-color="{t["shadow"]}" flood-opacity="{t["shadow_op"]}"/><feComposite in2="s" operator="in"/>
    <feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge>
  </filter>
  <filter id="glow" x="-80%" y="-80%" width="260%" height="260%">
    <feGaussianBlur stdDeviation="4.5" result="b"/>
    <feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
  </filter>
  <filter id="soft" x="-10%" y="-10%" width="120%" height="120%"><feGaussianBlur stdDeviation="3"/></filter>
</defs>''')
    # the tile: bevelled rim, then the face it frames
    o.append(f'<rect x="{FRAME}" y="{FRAME}" width="{S - 2 * FRAME}" height="{S - 2 * FRAME}" rx="{RX}" fill="url(#rim)" stroke="{t["outline"]}" stroke-width="1.2"/>')
    f = FRAME + RIM
    o.append(f'<rect x="{f}" y="{f}" width="{S - 2 * f}" height="{S - 2 * f}" rx="{RX - RIM}" fill="url(#face)"/>')
    # coordinates
    for i, ch in enumerate("ABCDEFGHIJ"):
        o.append(outline("mono", ch, 18, 65, X0 + (i + .5) * CELL + 6.5, t["label"], anchor="middle")[0])
    for i in range(10):
        o.append(outline("mono", str(i + 1), 18, X0 + (i + .5) * CELL, 68, t["label"], anchor="middle")[0])
    # the chart
    if t["edge_glow"]:
        o.append(f'<rect x="{X0}" y="{X0}" width="{b:.1f}" height="{b:.1f}" fill="none" stroke="{t["edge"]}" stroke-width="4" opacity="{t["edge_glow"]}" filter="url(#soft)"/>')
    o.append(f'<rect x="{X0}" y="{X0}" width="{b:.1f}" height="{b:.1f}" fill="url(#board)"/>')
    for k in range(1, 10):
        p = X0 + k * CELL
        o.append(f'<path d="M{p:.1f} {X0}V{X0 + b:.1f}M{X0} {p:.1f}H{X0 + b:.1f}" stroke="{t["grid"]}" stroke-width="1.2"/>')
    o.append(f'<rect x="{X0}" y="{X0}" width="{b:.1f}" height="{b:.1f}" fill="none" stroke="{t["edge"]}" stroke-width="1.6"/>')
    for (c, r) in MISSES:
        cx, cy = X0 + (c + .5) * CELL, X0 + (r + .5) * CELL
        glow = ' filter="url(#glow)"' if t["miss_glow"] else ""
        o.append(f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="3.9" fill="{t["miss"]}"{glow}/>')
    w = CELL - 2 * GAP
    rc, rr = RETICLE
    rcx, rcy = X0 + (rc + .5) * CELL, X0 + (rr + .5) * CELL
    o.append(f'<circle cx="{rcx:.1f}" cy="{rcy:.1f}" r="47" fill="url(#halo)"/>')
    o.append('<g filter="url(#shadow)">')
    for (c, r) in sorted(LIT - HITS):
        o.append(f'<rect x="{X0 + c * CELL + GAP:.1f}" y="{X0 + r * CELL + GAP:.1f}" width="{w:.1f}" height="{w:.1f}" rx="{TRX}" fill="url(#hull)"/>')
    o.append("</g>")
    # the reticle sits over the fleet, the hit it marks over the reticle
    y = "#ffbd07"
    o.append(f'<circle cx="{rcx:.1f}" cy="{rcy:.1f}" r="43" fill="none" stroke="{y}" stroke-width="1.4" opacity=".28"/>')
    o.append(f'<circle cx="{rcx:.1f}" cy="{rcy:.1f}" r="29.4" fill="none" stroke="{y}" stroke-width="3"/>')
    a, e = 17, 42.5
    o.append(f'<path d="M{rcx - e:.1f} {rcy:.1f}H{rcx - a:.1f}M{rcx + a:.1f} {rcy:.1f}H{rcx + e:.1f}'
             f'M{rcx:.1f} {rcy - e:.1f}V{rcy - a:.1f}M{rcx:.1f} {rcy + a:.1f}V{rcy + e:.1f}" stroke="{y}" stroke-width="3" stroke-linecap="round"/>')
    for (c, r) in sorted(HITS):
        o.append(f'<rect x="{X0 + c * CELL + GAP:.1f}" y="{X0 + r * CELL + GAP:.1f}" width="{w:.1f}" height="{w:.1f}" rx="{TRX}" fill="url(#hit)" filter="url(#glow)"/>')
    return "\n".join(o)

# --- the banner: the mark, the name beside it ---
BANNER = {
    "dark": dict(card=[(0, "#0d2840"), (1, "#050e1a")], edge="#1d4466", grid="#5b8fbf", grid_op=.16,
                 name="#eef4fa", line="#8fb6da", rule="#24496a"),
    "light": dict(card=[(0, "#ffffff"), (1, "#e3edf8")], edge="#c3d3e3", grid="#4a6d8e", grid_op=.12,
                  name="#10263b", line="#4a6d8e", rule="#c3d3e3"),
}

def banner(t, mt):
    W, H, PAD = 1280, 400, 44
    ms = H - 2 * PAD                                   # the mark, square, flush with the padding
    tx = PAD + ms + 44                                 # where the text column starts
    right = W - PAD - 12
    _, w100 = outline("display", "Broadside", 100, 0, 0, "none")
    size = min(200, (right - tx) / w100 * 100)         # the name fills the column
    name, _ = outline("display", "Broadside", size, tx, 228, t["name"])
    line, lw = outline("body", "EXACT BATTLESHIP ENGINE", 24, tx + 4, 296, t["line"], tracking=6)
    rule_x = tx + 4 + lw + 24
    return "\n".join([
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="Broadside, an exact Battleship engine">',
        "<title>Broadside</title>",
        f'''<defs>
  <radialGradient id="card" cx="{PAD + ms / 2}" cy="{H / 2}" r="{W * .8:.0f}" gradientUnits="userSpaceOnUse">{stops(t["card"])}</radialGradient>
  <pattern id="chart" width="40" height="40" patternUnits="userSpaceOnUse">
    <path d="M40 0V40M0 40H40" fill="none" stroke="{t["grid"]}" stroke-width="1.2"/>
  </pattern>
  <linearGradient id="fade" x1="0" y1="0" x2="1" y2="0"><stop offset=".42" stop-color="#fff" stop-opacity="0"/><stop offset="1" stop-color="#fff"/></linearGradient>
  <mask id="fademask"><rect width="{W}" height="{H}" fill="url(#fade)"/></mask>
  <clipPath id="cardclip"><rect width="{W}" height="{H}" rx="28"/></clipPath>
</defs>''',
        '<g clip-path="url(#cardclip)">',
        f'<rect width="{W}" height="{H}" fill="url(#card)"/>',
        f'<rect width="{W}" height="{H}" fill="url(#chart)" opacity="{t["grid_op"]}" mask="url(#fademask)"/>',
        "</g>",
        f'<rect x=".75" y=".75" width="{W - 1.5}" height="{H - 1.5}" rx="27.25" fill="none" stroke="{t["edge"]}" stroke-width="1.5"/>',
        f'<g transform="translate({PAD} {PAD}) scale({ms / S:.5f})">', mark(mt), "</g>",
        name, line,
        f'<rect x="{rule_x:.1f}" y="286" width="{right - rule_x:.1f}" height="2" fill="{t["rule"]}"/>',
        "</svg>"]) + "\n"

for name, theme in THEMES.items():
    open(f"{out}/logo-{name}.svg", "w").write(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}" width="{S}" height="{S}" role="img" aria-label="Broadside">\n'
        f"<title>Broadside</title>\n{mark(theme)}\n</svg>\n")
    open(f"{out}/banner-{name}.svg", "w").write(banner(BANNER[name], theme))
print("ok")
