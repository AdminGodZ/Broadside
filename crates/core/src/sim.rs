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

/// Plays `shooter` against `layout` to the end; returns every shot in order.
pub fn play_game(shooter: &mut dyn Shooter, layout: Layout, rng: &mut Rng) -> Vec<usize> {
    let mut board = Board::new(layout);
    let mut k = Knowledge::new();
    let mut shots = Vec::with_capacity(64);
    while !k.won() {
        let cell = shooter.choose(&k, rng);
        debug_assert!(
            k.unknown().has(cell),
            "{} fired twice at {cell}",
            shooter.name()
        );
        let outcome = board.fire(cell);
        k.record(cell, outcome);
        shots.push(cell);
    }
    shots
}

/// Shots `shooter` needs to sink `layout`.
pub fn play_out(shooter: &mut dyn Shooter, layout: Layout, rng: &mut Rng) -> u32 {
    play_game(shooter, layout, rng).len() as u32
}

pub type ShooterFactory<'a> = &'a (dyn Fn() -> Box<dyn Shooter> + Sync);
pub type PlacerFactory<'a> = &'a (dyn Fn() -> Box<dyn Placer> + Sync);

/// Runs `f(item, out)` for items `0..items` over `threads` threads, with one
/// `out` per thread; returns the per-thread outputs.
fn parallel<T: Send + Default>(
    items: u64,
    threads: usize,
    f: &(dyn Fn(u64, &mut T) + Sync),
    progress: &(dyn Fn(u64) + Sync),
) -> Vec<T> {
    let next = AtomicU64::new(0);
    let done = AtomicU64::new(0);
    thread::scope(|s| {
        let handles: Vec<_> = (0..threads.max(1))
            .map(|_| {
                s.spawn(|| {
                    let mut out = T::default();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= items {
                            break;
                        }
                        f(i, &mut out);
                        progress(done.fetch_add(1, Ordering::Relaxed) + 1);
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// Shots-to-win distribution of a shooter against a placer. Every game uses
/// fresh players, so nothing is learned between games.
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
        &|i, stats: &mut Stats| {
            let layout = placer().place(&mut Rng::stream(seed, 2 * i));
            stats.add(play_out(
                shooter().as_mut(),
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
    /// Wins over the second half of each match, after players had time to learn.
    pub a_wins_late: u64,
    pub b_wins_late: u64,
    pub a_stats: Stats,
    pub b_stats: Stats,
}

impl DuelResult {
    pub fn a_rate(&self) -> f64 {
        self.a_wins as f64 / (self.a_wins + self.b_wins) as f64
    }

    pub fn a_rate_late(&self) -> f64 {
        self.a_wins_late as f64 / (self.a_wins_late + self.b_wins_late) as f64
    }
}

/// Head-to-head games in matches of `match_len` games. Players persist (and
/// learn) through a match: shooters see each opponent layout afterwards,
/// placers see the shots fired at them. `match_len = 1` means fresh players
/// every game. Shooters alternate shots and never see the other side, so a
/// game is decided by comparing shots-to-win; the first mover (A on even
/// games, B on odd) wins ties.
pub fn duel(
    a: &Player,
    b: &Player,
    games: u64,
    match_len: u64,
    seed: u64,
    threads: usize,
    progress: &(dyn Fn(u64) + Sync),
) -> DuelResult {
    let match_len = match_len.max(1);
    let matches = games.div_ceil(match_len);
    let per_thread = parallel(
        matches,
        threads,
        &|m, r: &mut DuelResult| {
            let (mut sa, mut pa, mut sb, mut pb) =
                ((a.shooter)(), (a.placer)(), (b.shooter)(), (b.placer)());
            for g in 0..match_len {
                let i = m * match_len + g;
                if i >= games {
                    break;
                }
                let la = pa.place(&mut Rng::stream(seed, 4 * i));
                let lb = pb.place(&mut Rng::stream(seed, 4 * i + 1));
                let shots_a = play_game(sa.as_mut(), lb, &mut Rng::stream(seed, 4 * i + 2));
                let shots_b = play_game(sb.as_mut(), la, &mut Rng::stream(seed, 4 * i + 3));
                sa.learn(&lb);
                sb.learn(&la);
                pa.learn(&la, &shots_b);
                pb.learn(&lb, &shots_a);
                let (na, nb) = (shots_a.len() as u32, shots_b.len() as u32);
                r.a_stats.add(na);
                r.b_stats.add(nb);
                let a_won = na < nb || (na == nb && i.is_multiple_of(2));
                let late = 2 * g >= match_len;
                if a_won {
                    r.a_wins += 1;
                    r.a_wins_late += late as u64;
                } else {
                    r.b_wins += 1;
                    r.b_wins_late += late as u64;
                }
            }
        },
        progress,
    );
    let mut total = DuelResult::default();
    for r in &per_thread {
        total.a_wins += r.a_wins;
        total.b_wins += r.b_wins;
        total.a_wins_late += r.a_wins_late;
        total.b_wins_late += r.b_wins_late;
        total.a_stats.merge(&r.a_stats);
        total.b_stats.merge(&r.b_stats);
    }
    total
}
