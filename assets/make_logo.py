# Generates assets/logo.svg (the mark) and assets/banner.svg (the wordmark):
#   python3 assets/make_logo.py
# Letters are drawn as lit cells of a Battleship chart: the name is a fleet.
import sys
out = sys.argv[1] if len(sys.argv) > 1 else "assets"

FONT = {
 "B": ["11110","10001","10001","11110","10001","10001","11110"],
 "R": ["11110","10001","10001","11110","10100","10010","10001"],
 "O": ["01110","10001","10001","10001","10001","10001","01110"],
 "A": ["01110","10001","10001","11111","10001","10001","10001"],
 "D": ["11110","10001","10001","10001","10001","10001","11110"],
 "S": ["01111","10000","10000","01110","00001","00001","11110"],
 "I": ["111","010","010","010","010","010","111"],
 "E": ["11111","10000","10000","11110","10000","10000","11111"],
}
NAVY, NAVY2, SEA, LINE, INK, SOFT, FLAG, HIT = "#08131c", "#123349", "#0f2537", "#1e3e56", "#dce8ef", "#7f9bb0", "#f5c02a", "#f0503c"

def defs(uid):
    return f'''<defs>
  <radialGradient id="bg{uid}" cx="50%" cy="38%" r="75%">
    <stop offset="0" stop-color="{NAVY2}"/><stop offset="1" stop-color="{NAVY}"/>
  </radialGradient>
  <linearGradient id="hull{uid}" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#f4f9fc"/><stop offset="1" stop-color="#b9cbd7"/>
  </linearGradient>
  <radialGradient id="hit{uid}" cx="50%" cy="45%" r="60%">
    <stop offset="0" stop-color="#ffb199"/><stop offset=".45" stop-color="{HIT}"/><stop offset="1" stop-color="#9e2416"/>
  </radialGradient>
  <filter id="glow{uid}" x="-80%" y="-80%" width="260%" height="260%">
    <feGaussianBlur stdDeviation="6" result="b"/>
    <feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
  </filter>
  <linearGradient id="sweep{uid}" x1="0" y1="0" x2="1" y2="0">
    <stop offset="0" stop-color="{FLAG}" stop-opacity="0"/><stop offset="1" stop-color="{FLAG}" stop-opacity=".16"/>
  </linearGradient>
</defs>
<style>
  .ring{uid} {{ transform-box: fill-box; transform-origin: center; animation: ping{uid} 2.4s ease-out infinite; }}
  @keyframes ping{uid} {{ 0% {{ transform: scale(.6); opacity: .95; }} 80%,100% {{ transform: scale(2.1); opacity: 0; }} }}
  .scan{uid} {{ animation: scan{uid} 7s linear infinite; }}
  @keyframes scan{uid} {{ from {{ transform: translateX(0); }} to {{ transform: translateX(var(--travel)); }} }}
  @media (prefers-reduced-motion: reduce) {{ .ring{uid}, .scan{uid} {{ animation: none; }} }}
</style>'''

def board(x0, y0, cols, rows, size, lit, hits, misses, reticle, uid, gap):
    g = []
    # open sea and ruling
    g.append(f'<rect x="{x0}" y="{y0}" width="{cols*size}" height="{rows*size}" fill="{SEA}"/>')
    for c in range(cols + 1):
        g.append(f'<line x1="{x0+c*size}" y1="{y0}" x2="{x0+c*size}" y2="{y0+rows*size}" stroke="{LINE}" stroke-width="1"/>')
    for r in range(rows + 1):
        g.append(f'<line x1="{x0}" y1="{y0+r*size}" x2="{x0+cols*size}" y2="{y0+r*size}" stroke="{LINE}" stroke-width="1"/>')
    # a slow sonar sweep across the chart
    g.append(f'<clipPath id="clip{uid}"><rect x="{x0}" y="{y0}" width="{cols*size}" height="{rows*size}"/></clipPath>')
    # At rest (or without animation) the sweep waits off the chart, to its left.
    sw = cols * size * 0.3
    g.append(f'<g clip-path="url(#clip{uid})"><rect class="scan{uid}" style="--travel:{cols*size+sw:.0f}px" x="{x0-sw:.0f}" y="{y0}" width="{sw:.0f}" height="{rows*size}" fill="url(#sweep{uid})"/></g>')
    for (c, r) in misses:
        cx, cy = x0 + (c + .5) * size, y0 + (r + .5) * size
        g.append(f'<circle cx="{cx}" cy="{cy}" r="{size*.1:.1f}" fill="{SOFT}" opacity=".75"/>')
    for (c, r) in sorted(lit):
        x, y, w = x0 + c * size + gap, y0 + r * size + gap, size - 2 * gap
        if (c, r) in hits:
            g.append(f'<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="{size*.14:.1f}" fill="url(#hit{uid})" filter="url(#glow{uid})"/>')
        else:
            g.append(f'<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="{size*.14:.1f}" fill="url(#hull{uid})"/>')
    if reticle:
        c, r = reticle
        cx, cy, s = x0 + (c + .5) * size, y0 + (r + .5) * size, size
        g.append(f'<circle class="ring{uid}" cx="{cx}" cy="{cy}" r="{s*.62:.1f}" fill="none" stroke="{FLAG}" stroke-width="{max(1.5, s*.06):.1f}"/>')
        g.append(f'<circle cx="{cx}" cy="{cy}" r="{s*.78:.1f}" fill="none" stroke="{FLAG}" stroke-width="{max(1.2, s*.045):.1f}"/>')
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            a, b = s * .55, s * 1.08
            g.append(f'<line x1="{cx+dx*a:.1f}" y1="{cy+dy*a:.1f}" x2="{cx+dx*b:.1f}" y2="{cy+dy*b:.1f}" stroke="{FLAG}" stroke-width="{max(1.5, s*.06):.1f}" stroke-linecap="round"/>')
    return "\n".join(g)

def letter_cells(word, c0, r0):
    lit, c = set(), c0
    for ch in word:
        rows = FONT[ch]
        for r, line in enumerate(rows):
            for k, bit in enumerate(line):
                if bit == "1":
                    lit.add((c + k, r0 + r))
        c += len(rows[0]) + 1
    return lit

# --- the mark: a 10x10 chart with a B on it ---
size, x0, y0 = 40, 66, 66
lit = letter_cells("B", 2, 1)
hits = {(2, 4), (6, 2)}
mark = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512" role="img" aria-label="Broadside">',
        '<title>Broadside</title>', defs("m"),
        f'<rect width="512" height="512" rx="96" fill="url(#bgm)"/>',
        board(x0, y0, 10, 10, size, lit, hits, [(0, 8), (8, 6), (4, 9), (9, 1), (1, 3)], (6, 2), "m", 3)]
for i, ch in enumerate("ABCDEFGHIJ"):
    mark.append(f'<text x="{x0-14}" y="{y0+(i+.5)*size+4}" fill="{SOFT}" font-family="JetBrains Mono, ui-monospace, Menlo, monospace" font-size="11" text-anchor="middle">{ch}</text>')
for i in range(10):
    mark.append(f'<text x="{x0+(i+.5)*size}" y="{y0-12}" fill="{SOFT}" font-family="JetBrains Mono, ui-monospace, Menlo, monospace" font-size="11" text-anchor="middle">{i+1}</text>')
mark.append(f'<rect x="{x0}" y="{y0}" width="{10*size}" height="{10*size}" fill="none" stroke="{SOFT}" stroke-opacity=".55" stroke-width="2"/>')
mark.append("</svg>")
open(f"{out}/logo.svg", "w").write("\n".join(mark) + "\n")

# --- the banner: BROADSIDE spelled on a chart ---
word = "BROADSIDE"
width_cells = sum(len(FONT[c][0]) for c in word) + len(word) - 1
cols, rows, size = width_cells + 4, 11, 22
bw, bh = cols * size, rows * size
W, H = bw + 80, bh + 120
x0, y0 = 40, 40
lit = letter_cells(word, 2, 2)
hits = {(2, 5), (9, 2), (24, 8), (31, 3), (36, 5), (52, 2)} & lit
hits |= {(2, 5)}
misses = [(0, 0), (5, 10), (13, 0), (20, 10), (27, 0), (33, 10), (41, 0), (46, 10), (54, 0), (55, 9), (17, 5), (44, 4)]
misses = [m for m in misses if m not in lit and m[0] < cols]
reticle = sorted(hits)[-1]
ban = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="Broadside">',
       '<title>Broadside</title>', defs("b"),
       f'<rect width="{W}" height="{H}" rx="28" fill="url(#bgb)"/>',
       board(x0, y0, cols, rows, size, lit, hits, misses, reticle, "b", 2.5),
       f'<rect x="{x0}" y="{y0}" width="{bw}" height="{bh}" fill="none" stroke="{SOFT}" stroke-opacity=".5" stroke-width="1.5"/>',
       f'<text x="{x0}" y="{y0+bh+46}" fill="{INK}" font-family="JetBrains Mono, ui-monospace, Menlo, Consolas, monospace" font-size="17" letter-spacing="3.5" font-weight="600">EXACT BATTLESHIP ENGINE</text>',
       f'<text x="{x0+bw}" y="{y0+bh+46}" fill="{SOFT}" font-family="JetBrains Mono, ui-monospace, Menlo, Consolas, monospace" font-size="14" letter-spacing="2" text-anchor="end">30,093,975,536 FLEETS · EVERY SHOT</text>',
       "</svg>"]
open(f"{out}/banner.svg", "w").write("\n".join(ban) + "\n")
print("ok", cols, W, H, sorted(hits))
