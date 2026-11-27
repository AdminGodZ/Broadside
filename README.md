<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/banner-dark.svg">
    <img src="assets/banner-light.svg" alt="Broadside, an exact Battleship engine" width="100%">
  </picture>
</p>

<p align="center">
  <b>A Battleship engine that weighs every possible fleet on every shot.</b><br>
  Rust · exact Bayesian inference · endgame search · learns its opponent · plays in the browser
</p>

<p align="center">
  <a href="#play-it">Play it</a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="#how-strong-is-it">How strong is it</a> ·
  <a href="#commands">Commands</a> ·
  <a href="#roadmap">Roadmap</a>
</p>

---

<table>
<tr>
<td align="center" width="25%"><h3>44.0</h3>mean shots to sink a random fleet</td>
<td align="center" width="25%"><h3>30,093,975,536</h3>fleet layouts, all counted on every turn</td>
<td align="center" width="25%"><h3>~45 ms</h3>the full posterior of an empty board, one core</td>
<td align="center" width="25%"><h3>0.8</h3>shots per game better than the classic density bot</td>
</tr>
</table>

## Play it

**Online:** play at [broadside.admingod.ch](https://broadside.admingod.ch), or download the command-line
engine for macOS, Windows or Linux from the [latest release](https://github.com/AdminGodZ/Broadside/releases/latest).

**In the browser.** The whole engine compiles to WebAssembly and runs on your device, no server
needed. The game is the website's `/play` page; the build also folds it into one self-contained file
that works offline (the site's *Save it for offline play*):

```sh
cd site && npm ci && npm run build   # dist/play/ for the site, dist/broadside.html offline
open dist/broadside.html
```

Place your fleet (or roll a random one), win the coin toss, and fire. The engine learns through the
session where you hide ships and which waters you search last, so later games get harder.
Tick *Show where the engine expects your ships* to watch it think.

**In the terminal.**

```sh
cargo run --release -p broadside -- play --heat   # you vs the engine
cargo run --release -p broadside -- assist        # the engine calls your shots against anyone else
```

### Rules

Standard 10×10 board. Fleet: Carrier 5, Battleship 4, Cruiser 3, Submarine 3, Destroyer 2. Ships lie
horizontally or vertically, may touch, may not overlap. Each shot reports miss, hit, or *sunk* with
the ship's name. A hit alone does not say which ship it struck.

## How it works

<table>
<tr><td width="30%"><b>Exact inference</b></td><td>
Every layout consistent with the misses, hits and sinking reports (which ship sank, and when) is
counted exactly. Sunk ships are enumerated, hits are assigned to afloat ships by exact-cover search,
and ships touching no hit are counted in closed form with popcounts: all pairs minus overlapping
pairs. All 30 billion layouts of an empty board, with exact per-cell odds, take about 45 ms on one
core. Early positions are cached up to board symmetry.
</td></tr>
<tr><td><b>Shooting</b></td><td>
Fire at the likeliest cell until the ships still afloat have few enough joint placements to list
(200,000 by default; every position with two ships left has fewer than 30,000). From there the
engine searches. Each listed placement is a possible world, and a later position is just the subset
of worlds that agree with the shots since, so whole lines of play are judged by splitting lists,
with no further inference. The likeliest shots are compared by playing each to the end in every
world; positions with few worlds left are solved outright.
</td></tr>
<tr><td><b>A certain hit is not always right</b></td><td>
A cell can be sure to hold a ship while it is unclear which ship. Hitting it changes which later shot
announces which sinking, and that can cost information. The engine takes a certain hit without
thinking only when the same ship holds that cell in every layout; otherwise the search decides.
</td></tr>
<tr><td><b>Placement</b></td><td>
Tilted layouts: placements weighted per cell so ship occupancy hits a target between uniform and an
inverted centre bias. Shooters that assume uniform placement hunt the centre first and pay for it.
A single game uses an equilibrium mix of tilts, computed from the placement-game matrix, so even a
shooter that expects it cannot exploit it. (An equilibrium of the measured matrix of tilts against
shooters expecting a tilt, not of Battleship as a whole.)
</td></tr>
<tr><td><b>Learning across a match</b></td><td>
The shooter learns the opponent's placement habits by model selection (uniform, the tilt family,
shrunk per-cell occupancy, each scored on predicting the next revealed fleet), so it gains against
biased placers and loses nothing against uniform ones. The placer tracks where the opponent looks
last, and a bandit decides how hard to exploit it. Both learn only from what a real game shows: the
fleet revealed at the end, and the shots actually fired before the game ended.
</td></tr>
<tr><td><b>How the opponent spaces ships</b></td><td>
Many players never let ships touch; some stack them on purpose. Every layout is one of three kinds:
all ships apart, some meeting at a corner only, some lying side by side. The engine counts each kind
exactly and holds a belief over the three. It starts where random placement puts them (13%, 6%,
81%), so it assumes nothing about a new opponent. Hits and misses within a game correct the belief
by exact Bayes, and revealed fleets move it from game to game. Because an opponent may notice and
place against it, the learned belief is only trusted as far as it has predicted better than assuming
nothing: an opponent that always shows the kind of fleet the engine expects least gains nothing
measurable.
</td></tr>
</table>

## How strong is it

Mean shots to sink a uniformly placed fleet, 100,000 games each unless noted:

| Shooter | Mean shots | |
|---|---:|---|
| random | 95.4 | |
| hunt-target | 59.6 | |
| parity (checkerboard hunting) | 50.8 | |
| density (the classic probability-density bot) | 44.8 | |
| greedy (exact posterior, no search) | 44.60 ± 0.03 | 0.1 ms/game |
| broadside, search up to 30,000 placements | 44.15 ± 0.03 | 4.6 ms/game |
| **broadside, default: up to 200,000 placements** | **about 44.04** | 38 ms/game |

The default was measured against the 30,000 setting on the same 20,000 layouts: 0.11 ± 0.04 shots
better. Times are per game on a 24-core machine running one game per core. Single games swing by
about ±8.6 shots (one standard deviation), so one game between two good players is mostly luck;
a match is not.

### Opponents with a spacing habit

Shots saved per game by learning how the opponent spaces ships, against the same engine without it.
Matches of 20 games, 6,000 games per row, search up to 30,000 placements:

| The opponent's fleets | Shots saved per game |
|---|---:|
| ships never touch (`spaced`) | 4.33 ± 0.10 |
| spaced and stacked in turn, 5 games each (`swap5`) | 1.42 ± 0.12 |
| random placement (`uniform`) | 0.00 ± 0.08 |
| always the kind the engine expects least (`contrary`) | −0.03 ± 0.06 |
| the engine's own placement mix (`equilibrium`) | −0.10 ± 0.09 |

With the default search (3,000 games per row) the last rows read +0.18 ± 0.12 against random
placement and −0.13 ± 0.09 against the contrary placer: no cost that the runs can tell from zero.
A fixed assumption, by contrast, can be exploited. An engine told that ships lie apart nine times in
ten saves 5.7 shots a game against spaced fleets, and loses 1.0 against random placement and 2.0
against stacked fleets. That is why the belief starts neutral and is learned.

<sub>`broadside compare "broadside:exact=30000,weight=10000000" "broadside:exact=30000" --placer P
--match 20 --games 6000 --seed 44`; `weight=10000000` is an engine that never moves its belief.</sub>

### Head to head against a human-style player

The parity bot hunts on a checkerboard and finishes ships around its hits, much like a careful human.
Over 10,000 games Broadside won **70.8% ± 0.45** (7,076 to 2,924):

<p align="center">
  <img src="assets/duel-parity.svg" alt="Shots-to-win distributions of Broadside (mean 44.0) and the parity bot (mean 50.8), and the parity bot's chance to win matches of 1 to 51 games" width="100%">
</p>

| | Broadside | parity |
|---|---:|---:|
| Games won | 7,076 | 2,924 |
| Mean shots to win | 44.03 | 50.77 |
| Median (10th–90th percentile) | 43 (33–56) | 51 (39–62) |
| Fewest / most shots | 19 / 68 | 20 / 78 |

The two curves overlap so much that the weaker side still takes almost 3 games in 10. Skill, the 6.7
shot gap, accounts for about 23% of the outcome of a single game (the gap squared, as a share of
itself plus both sides' game-to-game variance). Over a match it adds up: the parity bot wins a
best-of-11 about 7 times in 100 and a best-of-51 less than once in 1,000.

<sub>`broadside duel broadside parity --games 10000 --seed 31 --dump FILE`, uniform placement, no
learning between games, 4 min 16 s on 24 cores. The chart is drawn by `assets/make_charts.py`
from `assets/data/duel-parity.csv`.</sub>

<details>
<summary><b>What the measurements say about the search</b></summary>

- **Reach is what pays.** Each step in how many placements the search will list (400, 3,000, 30,000,
  200,000) saved about another 0.1 shots; the cost grows about as fast as the limit.
- **Solving small positions optimally changes almost nothing.** With up to 24 placements left it
  altered the result of 135 games in 20,000 and the mean by 0.000 ± 0.001. Comparing shots by greedy
  play to the end is already close to optimal there. The solver stays on for the smallest positions,
  where it is free and its answers are provably right.
- **Thinking again at every later position helps, at a price.** With `deep=300` (lines of play
  through positions of up to 300 placements try the likeliest cells again instead of playing
  greedily) the 30,000 setting gained 0.07 ± 0.03 shots for ten times the time. Off by default.
- **Searching earlier does not pay.** With three ships afloat and too many placements to list,
  searching over 4,000 sampled placements (`afloat=3`) changed the mean by −0.01 ± 0.03 shots
  (1,000 samples: −0.02 ± 0.02), on 10,000 games each.
- **Playing for the win does not pay either.** With `race=16` the engine estimates how many shots the
  opponent still needs (rollouts of its play against our real fleet) and, while the race is open,
  picks the shot most likely to finish first. Against the plain engine, over 40,000 games each, it
  won 49.78%, 49.73% and 49.78% (± 0.25) for opponent spreads of 0, 1.5 and 3 shots, while needing
  0.2 more shots per game. Games between strong shooters are rarely decided by a shot that trades
  expected shots for variance.
- **A mean below 40 is not in sight.** From no search to the deepest search measured is 0.56 shots,
  and each further step costs about seven times the last one's time for a tenth of a shot.

</details>

<details>
<summary><b>Head-to-head matches</b> (older measurements)</summary>

`broadside` with its own placement, matches of 100 games so both sides may learn. Measured before the
search rewrite and under the old match rules, in which a placer saw every shot the opponent would
have fired, also after the game was over. A rerun under the current rules is pending.

| Opponent | Broadside wins |
|---|---:|
| density, uniform placement | 64.5% |
| density, edge-heavy placement | 72.0% |
| density, no-touch placement | 65.3% |
| greedy exact shooter, uniform placement | 60.6% |
| broadside without learning, uniform placement | 61.1% |
| broadside (self-play) | 49.9% |

</details>

## Commands

```sh
broadside play [--heat]          # you vs the engine; --heat shows its read of your fleet
broadside assist                 # engine advises shots vs any opponent; you report results
broadside sim --shooter S --placer P --games N     # shots-to-win statistics (--dump FILE: every game as CSV)
broadside compare A B [--placer P]                 # two shooters on the same layouts, game by game
                                                   #   (sim and compare take --match N: the players
                                                   #   keep what they learned for N games)
broadside duel A B [--placer-a P] [--placer-b P]   # head-to-head win rate (--match N: both sides learn;
                                                   #   --dump FILE: shots-to-win histograms as CSV)
broadside analyze --shooter S    # where the misses happen (hunt vs target, by ships afloat)
broadside matrix --alphas 0,1,2  # placement game: tilted placers vs shooters expecting them
broadside bench                  # time the exact posterior and the engine's moves
broadside count                  # brute-force count of all 30,093,975,536 layouts
```

Run with `cargo run --release -p broadside -- <command>`.

- **Shooters:** `broadside`, `greedy`, `density`, `NAME@tiltA`, `parity`, `hunt-target`, `random`, and
  `broadside:KEY=N,...` to tune the search (`exact`, `cands`, `solve`, `nodes`, `deep`, `afloat`,
  `samples`, `race`, `spread`; see `Search` in `crates/core/src/ai/engine.rs`) or its starting belief
  about the opponent's spacing (`apart`, `corners` in percent, `weight` in tenths of a game).
- **Placers:** `broadside`, `equilibrium`, `uniform`, `tiltA`, `mix:A=P,...`, and for testing the
  spacing belief `spaced` (no ships touch), `clumped` (every ship lies against another), `swapN`
  (spaced and clumped in turn, N games each) and `contrary` (always the kind of fleet the engine
  expects least).
- **Tests:** `cargo test`, slow ones with `cargo test --release -- --ignored`.

## Project layout

```
crates/
  core/   broadside-core: rules, bitboards, inference, search, placement (pure logic, no I/O)
  cli/    broadside: the command-line front end
  wasm/   broadside-wasm: the engine for the browser, a plain C ABI over WebAssembly
site/     the website and the browser game (Astro): npm run dev / npm run build → dist/, deployed
          by Vercel (vercel.json); the game is src/components/game, dist/broadside.html its offline file
assets/   logo and banner, light and dark, and charts (make_logo.py, make_charts.py draw them)
```

Boards are `u128` bitboards, bit `row * 10 + col`. Release builds use fat LTO and `target-cpu=native`.
The core has no dependencies.

## Releasing

1. Set `version` in `Cargo.toml` (workspace) and merge to `main`.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. `.github/workflows/release.yml` checks that the tag matches the version, runs the tests, builds the
   CLI for five platforms (with portable CPU baselines instead of `target-cpu=native`) and the browser
   game, and publishes a GitHub release with `SHA256SUMS.txt`. File names carry no version, so the
   website's `releases/latest/download/...` links always point at the newest release.

The website deploys from `main` on every push: Vercel runs `site/vercel-build.sh`, which installs Rust
and builds the Astro site in `site/` (its `prebuild` step builds the browser game into `/play`). Work on
it with `cd site && npm install && npm run dev`. `npm run test:e2e` plays the built site end to end
(Playwright, in Chromium, WebKit, Firefox and a phone; CI runs it on every pull request). The games replayed on the landing page come from
`cargo run --release -p broadside-core --example record_games > site/src/data/games.json`.

## Roadmap

- [x] Rules engine, baseline AIs, exact probability density, simulator, playable CLI
- [x] Endgame search, placement game, opponent modelling across a match
- [x] WebAssembly build and a browser UI
- [x] Opponents who keep ships apart, or stack them: exact counts per kind of layout, a learned
      belief that an opponent cannot milk
- [ ] An exact prior-weighted posterior before the search starts
- [ ] A shooter that tracks a mixed placer within a game
- [ ] Tests against outside engines and human players
- Tried, no gain measured (kept behind settings, off by default): search over sampled placements
  before the afloat ships can be listed (`afloat=N`), race-aware shooting (`race=N`)

## Reference numbers

| | |
|---|---|
| Positions per ship (5/4/3/3/2) | 120 / 140 / 160 / 160 / 180 |
| Fleet layouts, Cruiser ≠ Submarine | 30,093,975,536 |
| Fleet layouts, the two 3-ships interchangeable | 15,046,987,768 |
| Fleet layouts with no ships side by side | 5,797,542,752 |
| Fleet layouts with no ships touching, not even at corners | 3,851,502,784 |
| Minimum possible shots | 17 |

## License

[MIT](LICENSE). Use it, change it, ship it; keep the copyright notice.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
    <img src="assets/logo-light.svg" alt="" width="72">
  </picture>
</p>
