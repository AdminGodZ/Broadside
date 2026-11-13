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
broadside sim --shooter S --placer P --games N     # shots-to-win statistics
broadside duel A B [--placer-a P] [--placer-b P]   # head-to-head win rate
broadside analyze --shooter S    # where the misses happen (hunt vs target, by ships afloat)
broadside matrix --alphas 0,1,2  # placement game: tilted placers vs shooters expecting them
broadside bench                  # time the exact posterior on the empty board
broadside count                  # brute-force count of all 30,093,975,536 layouts
```

Run with `cargo run --release -p broadside -- <command>`. Shooters: `broadside`, `greedy`,
`density`, `density@tiltA`, `parity`, `hunt-target`, `random`. Placers: `uniform`, `spaced`, `tiltA`.

Tests: `cargo test`, slow ones with `cargo test --release -- --ignored`.

## How the engine works

- **Exact inference.** Every layout consistent with the misses, hits and "sunk X" reports
  (including which ship sank and when) is counted exactly: sunk-ship placements are
  enumerated, hits are assigned to afloat ships by exact-cover search, and ships touching no
  hit are counted in closed form (the last two via popcounts: all pairs minus overlapping
  pairs). The empty board, all 30 billion layouts with exact cell marginals, takes ~45 ms on
  one core. Early positions are cached up to board symmetry.
- **Shooting.** Fire at the most likely cell, except in the endgame, where rollout search
  plays the greedy policy to the end against every possible (or a sample of) placement of the
  afloat ships for each candidate shot, and deviates from greedy only for a real gain.
- **Placement.** Tilted layouts: placements weighted per cell so ship occupancy hits a target
  between uniform and inverted centre bias. Shooters that assume uniform placement hunt the
  centre first and pay for it; the tilt is chosen with the placement-game matrix so a shooter
  that expects it cannot exploit it much.

## Rules (standard)

- 10x10 board. Fleet: Carrier 5, Battleship 4, Cruiser 3, Submarine 3, Destroyer 2.
- Ships are horizontal or vertical, may touch, may not overlap.
- Each shot reports miss / hit / sunk (with the ship's identity).

## Roadmap

1. ~~Rules engine~~, ~~baseline AIs~~, ~~exact probability density~~, ~~simulator~~, ~~playable CLI~~.
2. **Stronger play** (in progress): endgame rollout search (done), placement game (done, tuning),
   deeper search earlier in the game, opponent modeling across a match (needs a weighted
   posterior), race-aware shooting.
3. **Later, maybe**: WebAssembly build for a browser UI; self-play toward an unexploitable strategy.

## Reference numbers

| | |
|---|---|
| Positions per ship (5/4/3/3/2) | 120 / 140 / 160 / 160 / 180 |
| Fleet layouts, Cruiser != Submarine | 30,093,975,536 |
| Fleet layouts, 3-ships interchangeable | 15,046,987,768 |
| Minimum possible shots | 17 |

Mean shots to sink a uniformly placed fleet (10,000+ games):

| Shooter | Mean |
|---|---|
| random | 95.4 |
| hunt-target | 59.6 |
| parity | 50.8 |
| density (classic probability density) | 44.8 |
| greedy (exact posterior) | 44.5 |
| broadside (exact posterior + endgame rollouts) | 44.3 |
