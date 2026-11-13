//! Simulation: many games across threads, reproducible from a seed.
//!
//! Game `i` always uses the same random streams, so results do not depend on
//! thread count or scheduling.

use crate::ai::{Placer, Shooter};
use crate::bitboard::CELLS;
use crate::game::{Board, Knowledge, Layout};
use crate::rng::Rng;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

/// Distribution of shots needed to sink a whole fleet.
#[derive(Clone, Debug)]
pub struct Stats {
    pub hist: [u64; CELLS + 1],
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            hist: [0; CELLS + 1],
        }
    }
}

impl Stats {
    pub fn add(&mut self, shots: u32) {
        self.hist[shots as usize] += 1;
    }

    pub fn merge(&mut self, other: &Stats) {
        for (a, b) in self.hist.iter_mut().zip(other.hist.iter()) {
            *a += b;
        }
    }

    pub fn games(&self) -> u64 {
        self.hist.iter().sum()
    }

    pub fn mean(&self) -> f64 {
        let n = self.games() as f64;
        self.hist
            .iter()
            .enumerate()
            .map(|(s, &c)| s as f64 * c as f64)
            .sum::<f64>()
            / n
    }

    pub fn std_dev(&self) -> f64 {
        let (n, m) = (self.games() as f64, self.mean());
        let var = self
            .hist
            .iter()
            .enumerate()
            .map(|(s, &c)| (s as f64 - m).powi(2) * c as f64)
            .sum::<f64>();
        (var / (n - 1.0).max(1.0)).sqrt()
    }

    /// Standard error of the mean.
    pub fn sem(&self) -> f64 {
        self.std_dev() / (self.games() as f64).sqrt()
    }

    /// Smallest shot count reached by at least fraction `q` of games.
    pub fn quantile(&self, q: f64) -> u32 {
        let target = (q * self.games() as f64).ceil().max(1.0) as u64;
        let mut acc = 0;
        for (s, &c) in self.hist.iter().enumerate() {
            acc += c;
            if acc >= target {
                return s as u32;
            }
        }
        CELLS as u32
    }

    pub fn min(&self) -> u32 {
        self.hist.iter().position(|&c| c > 0).unwrap_or(0) as u32
    }

    pub fn max(&self) -> u32 {
        self.hist.iter().rposition(|&c| c > 0).unwrap_or(0) as u32
    }
}

/// Shots `shooter` needs to sink `layout`.
pub fn play_out(shooter: &mut dyn Shooter, layout: Layout, rng: &mut Rng) -> u32 {
    let mut board = Board::new(layout);
    let mut k = Knowledge::new();
    while !k.won() {
        let cell = shooter.choose(&k, rng);
        debug_assert!(
            k.unknown().has(cell),
            "{} fired twice at {cell}",
            shooter.name()
        );
        let outcome = board.fire(cell);
        k.record(cell, outcome);
    }
    shooter.learn(&layout);
    k.shots_fired()
}

pub type ShooterFactory<'a> = &'a (dyn Fn() -> Box<dyn Shooter> + Sync);
pub type PlacerFactory<'a> = &'a (dyn Fn() -> Box<dyn Placer> + Sync);

/// Runs `f(game_index, state)` for games `0..games` over `threads` threads,
/// with one `state` per thread from `init`; merges the per-thread results.
fn parallel<T: Send + Default, S>(
    games: u64,
    threads: usize,
    init: &(dyn Fn() -> S + Sync),
    f: &(dyn Fn(u64, &mut S, &mut T) + Sync),
    progress: &(dyn Fn(u64) + Sync),
) -> Vec<T> {
    let next = AtomicU64::new(0);
    let done = AtomicU64::new(0);
    thread::scope(|s| {
        let handles: Vec<_> = (0..threads.max(1))
            .map(|_| {
                s.spawn(|| {
                    let mut state = init();
                    let mut out = T::default();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= games {
                            break;
                        }
                        f(i, &mut state, &mut out);
                        progress(done.fetch_add(1, Ordering::Relaxed) + 1);
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// Shots-to-win distribution of a shooter against a placer.
pub fn benchmark(
    shooter: ShooterFactory,
    placer: PlacerFactory,
    games: u64,
    seed: u64,
    threads: usize,
    progress: &(dyn Fn(u64) + Sync),
) -> Stats {
    let per_thread = parallel(
        games,
        threads,
        &|| (shooter(), placer()),
        &|i, (sh, pl): &mut (Box<dyn Shooter>, Box<dyn Placer>), stats: &mut Stats| {
            let layout = pl.place(&mut Rng::stream(seed, 2 * i));
            stats.add(play_out(
                sh.as_mut(),
                layout,
                &mut Rng::stream(seed, 2 * i + 1),
            ));
        },
        progress,
    );
    let mut total = Stats::default();
    for s in &per_thread {
        total.merge(s);
    }
    total
}

/// One side of a duel.
pub struct Player<'a> {
    pub shooter: ShooterFactory<'a>,
    pub placer: PlacerFactory<'a>,
}

#[derive(Clone, Debug, Default)]
pub struct DuelResult {
    pub a_wins: u64,
    pub b_wins: u64,
    pub a_stats: Stats,
    pub b_stats: Stats,
}

impl DuelResult {
    pub fn a_rate(&self) -> f64 {
        self.a_wins as f64 / (self.a_wins + self.b_wins) as f64
    }
}

/// Head-to-head games. Shooters alternate shots and never see the other side,
/// so each game is decided by comparing shots-to-win; the first mover (A on
/// even games, B on odd) wins ties.
pub fn duel(
    a: &Player,
    b: &Player,
    games: u64,
    seed: u64,
    threads: usize,
    progress: &(dyn Fn(u64) + Sync),
) -> DuelResult {
    let per_thread = parallel(
        games,
        threads,
        &|| ((a.shooter)(), (a.placer)(), (b.shooter)(), (b.placer)()),
        &|i, (sa, pa, sb, pb), r: &mut DuelResult| {
            let la = pa.place(&mut Rng::stream(seed, 4 * i));
            let lb = pb.place(&mut Rng::stream(seed, 4 * i + 1));
            let na = play_out(sa.as_mut(), lb, &mut Rng::stream(seed, 4 * i + 2));
            let nb = play_out(sb.as_mut(), la, &mut Rng::stream(seed, 4 * i + 3));
            r.a_stats.add(na);
            r.b_stats.add(nb);
            let a_first = i % 2 == 0;
            if na < nb || (na == nb && a_first) {
                r.a_wins += 1;
            } else {
                r.b_wins += 1;
            }
        },
        progress,
    );
    let mut total = DuelResult::default();
    for r in &per_thread {
        total.a_wins += r.a_wins;
        total.b_wins += r.b_wins;
        total.a_stats.merge(&r.a_stats);
        total.b_stats.merge(&r.b_stats);
    }
    total
}
