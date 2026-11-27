# Broadside

Battleship engine in Rust. Goal: the strongest engine practical, and as fast as possible.
See README.md for the roadmap and reference numbers.

## Decisions

- Rust for speed (no GC, safe multithreading, can target WASM later).
- Boards are `Bitboard(u128)`, bit index `row * 10 + col`. Prefer bit ops over arrays in hot paths.
- `broadside-core` stays pure (no I/O, no printing) so CLI, simulator and a future WASM build share it.
- Release profile uses fat LTO, codegen-units=1, panic=abort; `.cargo/config.toml` sets `target-cpu=native`.
- No dependencies yet. Add crates deliberately (rayon for parallelism and a fast RNG are the likely first ones).
- Measure, don't guess: any AI change gets judged by simulator results, any perf change by benchmarks.

## Commands

- `cargo test` / `cargo test --release -- --ignored` (slow tests)
- `cargo run --release -p broadside -- count`
- `cargo run --release -p broadside -- compare A B` (paired comparison of two shooters)
- `cargo clippy --all-targets`
- `cd site && npm run build` (website and browser game; the engine as WebAssembly, and the game folded into `site/dist/broadside.html` for offline play)
- `cd site && npm run test:e2e` (after a build: the site and the game played end to end in Chromium, WebKit, Firefox and a phone; `npx playwright install` first)
