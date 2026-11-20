# Draws assets/duel-parity.svg from assets/data/duel-parity.csv, the shots-to-win
# histograms of `broadside duel broadside parity --games 10000 --seed 31 --dump`:
#   python3 assets/make_charts.py
import csv, math

rows = [tuple(map(int, r)) for r in list(csv.reader(open("assets/data/duel-parity.csv")))[1:]]
na, nb = sum(r[1] for r in rows), sum(r[2] for r in rows)
pa = {s: a / na for s, a, _ in rows}
pb = {s: b / nb for s, _, b in rows}
mean = lambda p: sum(s * v for s, v in p.items())
sd = lambda p: math.sqrt(sum((s - mean(p)) ** 2 * v for s, v in p.items()))
ma, mb, sa, sb = mean(pa), mean(pb), sd(pa), sd(pb)
# Parity wins a game when it needs fewer shots; ties go to the first mover.
p = sum(pa[x] * pb[y] for x in pa for y in pb if y < x) + sum(pa[x] * pb.get(x, 0) for x in pa) / 2
match = lambda n: sum(math.comb(n, k) * p**k * (1 - p) ** (n - k) for k in range(n // 2 + 1, n + 1))
skill = (mb - ma) ** 2 / ((mb - ma) ** 2 + sa**2 + sb**2)

NAVY, NAVY2, LINE, INK, SOFT = "#08131c", "#123349", "#1e3e56", "#dce8ef", "#7f9bb0"
A, B = "#3987e5", "#d95926"  # validated pair on the navy surface (OKLab CVD ΔE 26.8)
FONT = "Inter, -apple-system, Segoe UI, Helvetica, Arial, sans-serif"
W, H = 1290, 590
o = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="Broadside against the parity bot: 10,000 games">',
     "<title>Broadside vs parity, 10,000 games</title>",
     f'<defs><radialGradient id="bg" cx="50%" cy="30%" r="80%"><stop offset="0" stop-color="{NAVY2}"/><stop offset="1" stop-color="{NAVY}"/></radialGradient></defs>',
     f'<rect width="{W}" height="{H}" rx="28" fill="url(#bg)"/>']
t = lambda x, y, s, size=13, fill=SOFT, anchor="start", weight=400, font=FONT, ls=0: o.append(
    f'<text x="{x:.1f}" y="{y:.1f}" fill="{fill}" font-family="{font}" font-size="{size}" font-weight="{weight}" text-anchor="{anchor}" letter-spacing="{ls}">{s}</text>')

t(40, 52, "Broadside against the parity bot, 10,000 games", 16, INK, weight=600)
t(40, 76, "Uniformly placed fleets, no learning between games, first move alternating.", 13)
# headline figures
for i, (big, small) in enumerate([(f"{100 * 7076 / 10000:.1f}%", "Broadside wins (± 0.45)"),
                                  (f"{ma:.1f} vs {mb:.1f}", "mean shots to win"),
                                  (f"±{(sa + sb) / 2:.1f}", "shots, game to game"),
                                  (f"≈ {100 * skill:.0f}%", "of the outcome is skill")]):
    x = 40 + i * 302
    t(x, 128, big, 34, INK, weight=600)
    t(x, 150, small, 13)

# left: shots-to-win distribution
L, T, R, Bm = 70, 200, 760, 500
X = lambda s: L + (s - 17) / (80 - 17) * (R - L)
Y = lambda v: Bm - v / 0.05 * (Bm - T)
t(40, 184, "Shots needed to sink the fleet (share of games)", 14, INK, weight=600)
for k in range(6):
    o.append(f'<line x1="{L}" x2="{R}" y1="{Y(k / 100):.1f}" y2="{Y(k / 100):.1f}" stroke="{LINE}"/>')
    t(L - 8, Y(k / 100) + 4, f"{k}%", 11, anchor="end")
for s in range(20, 81, 10):
    t(X(s), Bm + 20, s, 11, anchor="middle")
for dist, color in ((pa, A), (pb, B)):
    pts = " ".join(f"L{X(s):.1f},{Y(dist.get(s, 0)):.1f}" for s in range(17, 81))
    o.append(f'<path d="M{X(17):.1f},{Y(0):.1f} {pts} L{X(80):.1f},{Y(0):.1f} Z" fill="{color}" fill-opacity=".13"/>')
    o.append(f'<path d="M{X(17):.1f},{Y(0):.1f} {pts}" fill="none" stroke="{color}" stroke-width="2.2" stroke-linejoin="round"/>')
# Mean lines, labelled on their outer sides so the labels never cross a line.
for m, color, label, side in ((ma, A, f"Broadside · mean {ma:.1f}", -1), (mb, B, f"parity · mean {mb:.1f}", 1)):
    o.append(f'<line x1="{X(m):.1f}" x2="{X(m):.1f}" y1="{T}" y2="{Bm}" stroke="{color}" stroke-width="1.5" stroke-dasharray="4 4"/>')
    o.append(f'<rect x="{X(m) + side * 8 - (10 if side < 0 else 0):.1f}" y="{T + 2}" width="10" height="3" rx="1.5" fill="{color}"/>')
    t(X(m) + side * 23, T + 8, label, 12, INK, anchor="end" if side < 0 else "start")

# right: parity's chance to win a match
L2, R2 = 830, 1250
t(810, 184, "Parity's chance to win a match", 14, INK, weight=600)
ns = [1, 3, 5, 7, 11, 21, 51]
Y2 = lambda v: Bm - v / 0.30 * (Bm - T)
for v in (0, .1, .2, .3):
    o.append(f'<line x1="{L2}" x2="{R2}" y1="{Y2(v):.1f}" y2="{Y2(v):.1f}" stroke="{LINE}"/>')
    t(L2 - 8, Y2(v) + 4, f"{v * 100:.0f}%", 11, anchor="end")
slot = (R2 - L2) / len(ns)
for i, n in enumerate(ns):
    v, cx, bw = match(n), L2 + slot * (i + .5), 34
    h = max(1.5, Bm - Y2(v))
    r = min(4, h)
    x, y = cx - bw / 2, Bm - h
    o.append(f'<path d="M{x:.1f},{Bm} V{y + r:.1f} Q{x:.1f},{y:.1f} {x + r:.1f},{y:.1f} H{x + bw - r:.1f} Q{x + bw:.1f},{y:.1f} {x + bw:.1f},{y + r:.1f} V{Bm} Z" fill="{B}"/>')
    t(cx, y - 8, f"{100 * v:.1f}%" if v >= .01 else f"{100 * v:.2f}%", 12, INK, anchor="middle")
    t(cx, Bm + 20, "1 game" if n == 1 else f"best of {n}", 11, anchor="middle")

t(40, 566, "The curves overlap heavily, so one game is mostly luck; over a match the 6.7-shot gap decides. "
           "Skill share: the mean gap squared over itself plus both sides' variance.", 12)
o.append("</svg>")
open("assets/duel-parity.svg", "w").write("\n".join(o) + "\n")
print(f"p={p:.4f} skill={skill:.3f}")
