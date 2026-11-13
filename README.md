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
cargo run --release -p broadside -- count     # count all 30,093,975,536 fleet layouts
cargo test                                     # fast tests
cargo test --release -- --ignored              # slow tests (full layout count)
cargo clippy --all-targets
```

## Rules (standard)

- 10x10 board. Fleet: Carrier 5, Battleship 4, Cruiser 3, Submarine 3, Destroyer 2.
- Ships are horizontal or vertical, may touch, may not overlap.
- Each shot reports miss / hit / sunk (with the ship's identity).

## Roadmap

1. **Rules engine**: game state, shot resolution, sinking, win detection, random legal placement.
2. **Baseline AIs**: random, hunt/target, parity hunt/target.
3. **Probability density**: exact counting of layouts consistent with every hit, miss and sunk ship. Fire at the hottest cell.
4. **Simulator**: multithreaded AI-vs-AI runs. Report mean / median / distribution of shots to win and head-to-head win rates.
5. **Playable CLI**: human vs engine in the terminal, with an optional probability heatmap.
6. **Stronger play**: lookahead / information gain, win-probability (race-aware) shooting, opponent modeling, smarter placement.
7. **Later, maybe**: WebAssembly build for a browser UI; self-play / regret minimization toward an unexploitable strategy.

## Reference numbers

| | |
|---|---|
| Positions per ship (5/4/3/3/2) | 120 / 140 / 160 / 160 / 180 |
| Fleet layouts, Cruiser != Submarine | 30,093,975,536 |
| Fleet layouts, 3-ships interchangeable | 15,046,987,768 |
| Random shooting, mean shots to win | ~95 |
| Good probability-density AI | ~40-45 |
| Minimum possible | 17 |
