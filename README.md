# Broadside

A Battleship engine built to be as strong and as fast as possible, in Rust.

## Layout

```
crates/
  core/   broadside-core: rules, bitboards, AI (pure logic, no I/O)
  cli/    broadside: command-line front end (count / play / sim)
```

## Commands

```sh
broadside play [--heat]          # you vs the engine; --heat shows its read of your fleet
broadside assist                 # engine advises shots vs any opponent; you report results
broadside sim --shooter S --placer P --games N     # shots-to-win statistics (--dump FILE: every game as CSV)
broadside compare A B [--placer P]                 # two shooters on the same layouts, game by game
broadside duel A B [--placer-a P] [--placer-b P]   # head-to-head win rate (--match N: both sides learn)
broadside analyze --shooter S    # where the misses happen (hunt vs target, by ships afloat)
broadside matrix --alphas 0,1,2  # placement game: tilted placers vs shooters expecting them
broadside bench                  # time the exact posterior and the engine's moves
broadside count                  # brute-force count of all 30,093,975,536 layouts
```

Run with `cargo run --release -p broadside -- <command>`. Shooters: `broadside`, `greedy`,
`density`, `NAME@tiltA`, `parity`, `hunt-target`, `random`, and `broadside:KEY=N,...` to set
search limits (see `Search` in `crates/core/src/ai/engine.rs`). Placers: `broadside`,
`equilibrium`, `uniform`, `spaced`, `tiltA`, `mix:A=P,...`.

Tests: `cargo test`, slow ones with `cargo test --release -- --ignored`.

## How the engine works

- **Exact inference.** Every layout consistent with the misses, hits and "sunk X" reports
  (including which ship sank and when) is counted exactly: sunk-ship placements are
  enumerated, hits are assigned to afloat ships by exact-cover search, and ships touching no
  hit are counted in closed form (the last two via popcounts: all pairs minus overlapping
  pairs). The empty board, all 30 billion layouts with exact cell marginals, takes ~45 ms on
  one core. Early positions are cached up to board symmetry.
- **Shooting.** Fire at the most likely cell until the afloat ships have few enough joint
  placements to list (200,000 by default; every position with two ships left has fewer than 30,000).
  From there the engine searches. Each listed placement is a possible world, and a later
  position is just the subset of worlds that agree with the shots since, so whole lines of
  play are evaluated by splitting lists, with no further inference. The likeliest shots are
  compared by playing each to the end over every world; positions with few worlds left are
  solved for the optimal shot.
- **A certain hit is not always right.** A cell can be sure to hold a ship while it is unclear
  which ship. Hitting it changes which later shot announces which sinking, and that can cost
  information. The engine takes a certain hit without thinking only when the same ship holds
  the cell in every layout; otherwise the search decides.
- **Placement.** Tilted layouts: placements weighted per cell so ship occupancy hits a target
  between uniform and inverted centre bias. Shooters that assume uniform placement hunt the
  centre first and pay for it. A single game uses an equilibrium mix of tilts, computed from
  the placement-game matrix, so even a shooter that expects it cannot exploit it. (That is an
  equilibrium of the measured matrix of tilts against shooters expecting a tilt, not of
  Battleship as a whole.)
- **Learning across a match.** The shooter learns the opponent's placement habits by model
  selection (uniform, tilt family, shrunk per-cell occupancy, each scored on predicting the next
  revealed fleet), so it gains against biased placers and loses nothing against uniform ones.
  Under a learned prior the search weighs worlds exactly; before the search starts the prior
  enters approximately. The placer tracks where the opponent looks last and a bandit decides
  how hard to exploit it. It sees only the shots actually fired: a game ends when either fleet
  is sunk.

## Rules (standard)

- 10x10 board. Fleet: Carrier 5, Battleship 4, Cruiser 3, Submarine 3, Destroyer 2.
- Ships are horizontal or vertical, may touch, may not overlap.
- Each shot reports miss / hit / sunk (with the ship's identity).

## Roadmap

1. ~~Rules engine~~, ~~baseline AIs~~, ~~exact probability density~~, ~~simulator~~, ~~playable CLI~~.
2. ~~Endgame search~~, ~~placement game~~, ~~opponent modelling across a match~~.
3. **Open**: search before the afloat ships can be listed (three or more ships in open water;
   sampled rollouts exist behind `afloat=N` but are unproven), race-aware shooting (play for the
   win, not the fewest shots, once the opponent is close), an exact prior-weighted posterior
   before the search starts, a shooter that tracks a mixed placer within a game.
4. **Later, maybe**: WebAssembly build for a browser UI.

## Reference numbers

| | |
|---|---|
| Positions per ship (5/4/3/3/2) | 120 / 140 / 160 / 160 / 180 |
| Fleet layouts, Cruiser != Submarine | 30,093,975,536 |
| Fleet layouts, 3-ships interchangeable | 15,046,987,768 |
| Minimum possible shots | 17 |

Mean shots to sink a uniformly placed fleet (100,000 games each unless noted):

| Shooter | Mean |
|---|---|
| random | 95.4 |
| hunt-target | 59.6 |
| parity | 50.8 |
| density (classic probability density) | 44.8 |
| greedy (exact posterior, no search) | 44.60 ± 0.03 |
| broadside, search up to 30,000 placements (4.6 ms/game) | 44.15 ± 0.03 |
| broadside, default: up to 200,000 placements (38 ms/game) | about 44.04 |

The default was measured against the 30,000 setting on the same 20,000 layouts: 0.11 ± 0.04
shots better (44.12 against 44.24 on those games). Times are per game on a 24-core machine
running one game per core.

What the measurements say about the search:

- Reach is what pays. Each step in how many placements the search will list (400, 3,000,
  30,000, 200,000) saved about another 0.1 shots; the cost grows about as fast as the limit.
- Solving small positions optimally changes almost nothing: with up to 24 placements left it
  altered the result of 135 games in 20,000 and the mean by 0.000 ± 0.001. Comparing shots by
  greedy play to the end is already close to optimal there. The solver stays on for the
  smallest positions, where it is free and its answers are provably right.
- Thinking again at every later position helps, at a price: with `deep=300` (lines of play
  through positions of up to 300 placements try the likeliest cells again instead of playing
  greedily) the 30,000 setting gained 0.07 ± 0.03 shots for ten times the time. Off by default.
- A mean below 40 is not in sight. From no search to the deepest search measured is 0.56
  shots, and each further step costs about seven times the last one's time for a tenth of a
  shot.

Head-to-head (`broadside` with its own placement, matches of 100 games so both sides may
learn). These were measured before the search rewrite and under the old match rules, in which
a placer saw every shot the opponent would have fired, also after the game was over. A rerun
under the current rules is pending.

| Opponent | Broadside wins |
|---|---|
| density, uniform placement | 64.5% |
| density, edge-heavy placement | 72.0% |
| density, no-touch placement | 65.3% |
| greedy exact shooter, uniform placement | 60.6% |
| broadside without learning, uniform placement | 61.1% |
| broadside (self-play) | 49.9% |
