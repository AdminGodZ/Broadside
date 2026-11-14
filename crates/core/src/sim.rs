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
/// Panics if the shooter fires off the board or at a cell twice: a score
/// from an illegal game would mean nothing.
pub fn play_game(shooter: &mut dyn Shooter, layout: Layout, rng: &mut Rng) -> Vec<usize> {
    let mut board = Board::new(layout);
    let mut k = Knowledge::new();
    let mut shots = Vec::with_capacity(64);
    while !k.won() {
        let cell = shooter.choose(&k, rng);
        assert!(
            cell < CELLS && k.unknown().has(cell),
            "{} fired at {cell}: off the board or already shot",
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

/// One simulated game: the layout and the shots it took to sink.
#[derive(Clone, Copy, Debug)]
pub struct Game {
    pub layout: Layout,
    pub shots: u32,
}

/// Plays `games` games of a shooter against a placer and returns them in
/// order. Every game uses fresh players, so nothing is learned between games,
/// and game `i` sees the same layout whatever the shooter: results of two
/// shooters pair up game by game.
pub fn games(
    shooter: ShooterFactory,
    placer: PlacerFactory,
    games: u64,
    seed: u64,
    threads: usize,
    progress: &(dyn Fn(u64) + Sync),
) -> Vec<Game> {
    let per_thread = parallel(
        games,
        threads,
        &|i, out: &mut Vec<(u64, Game)>| {
            let layout = placer().place(&mut Rng::stream(seed, 2 * i));
            let shots = play_out(
                shooter().as_mut(),
                layout,
                &mut Rng::stream(seed, 2 * i + 1),
            );
            out.push((i, Game { layout, shots }));
        },
        progress,
    );
    let mut all: Vec<(u64, Game)> = per_thread.into_iter().flatten().collect();
    all.sort_unstable_by_key(|g| g.0);
    all.into_iter().map(|g| g.1).collect()
}

/// Shots-to-win distribution of a shooter against a placer (see `games`).
pub fn benchmark(
    shooter: ShooterFactory,
    placer: PlacerFactory,
    games: u64,
    seed: u64,
    threads: usize,
    progress: &(dyn Fn(u64) + Sync),
) -> Stats {
    let mut stats = Stats::default();
    for g in self::games(shooter, placer, games, seed, threads, progress) {
        stats.add(g.shots);
    }
    stats
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
    /// (A's wins, games) of every match. Games within a match depend on each
    /// other once players learn, so matches are the unit for error bars.
    pub matches: Vec<(u32, u32)>,
    /// Shots each side needed to sink the other's fleet, played out to the
    /// end even where the real game was already decided.
    pub a_stats: Stats,
    pub b_stats: Stats,
}

impl DuelResult {
    pub fn a_rate(&self) -> f64 {
        self.a_wins as f64 / (self.a_wins + self.b_wins) as f64
    }

    /// `None` if no game fell into the second half of its match.
    pub fn a_rate_late(&self) -> Option<f64> {
        let n = self.a_wins_late + self.b_wins_late;
        (n > 0).then(|| self.a_wins_late as f64 / n as f64)
    }

    /// Standard error of `a_rate`, from the spread between matches.
    pub fn a_rate_se(&self) -> f64 {
        let n = self.matches.len() as f64;
        let rate = self.a_rate();
        let games = (self.a_wins + self.b_wins) as f64;
        // Ratio estimator over matches (they may differ in length).
        let ss: f64 = self
            .matches
            .iter()
            .map(|&(w, g)| (w as f64 - rate * g as f64).powi(2))
            .sum();
        (n / (n - 1.0).max(1.0) * ss).sqrt() / games
    }
}

/// Head-to-head games in matches of `match_len` games. Players persist (and
/// learn) through a match: after each game shooters see the opponent's
/// layout, and placers see the shots fired at their own fleet before the game
/// ended. `match_len = 1` means fresh players every game.
///
/// Shooters alternate shots and never see the other side, so a game is
/// decided by comparing shots-to-win, the first mover winning ties. The first
/// move alternates within a match and between matches, so neither half of a
/// match favours a side.
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
            let mut tally = (0u32, 0u32);
            for g in 0..match_len {
                let i = m * match_len + g;
                if i >= games {
                    break;
                }
                let la = pa.place(&mut Rng::stream(seed, 4 * i));
                let lb = pb.place(&mut Rng::stream(seed, 4 * i + 1));
                let shots_a = play_game(sa.as_mut(), lb, &mut Rng::stream(seed, 4 * i + 2));
                let shots_b = play_game(sb.as_mut(), la, &mut Rng::stream(seed, 4 * i + 3));
                let (na, nb) = (shots_a.len(), shots_b.len());
                let a_first = (m + g).is_multiple_of(2);
                let a_won = na < nb || (na == nb && a_first);
                // The game stops at the winner's last shot: the loser has
                // fired as many shots, or one fewer if the winner moved first.
                let (fired_a, fired_b) = if a_won {
                    (na, na - a_first as usize)
                } else {
                    (nb - !a_first as usize, nb)
                };
                sa.learn(&lb);
                sb.learn(&la);
                pa.learn(&la, &shots_b[..fired_b]);
                pb.learn(&lb, &shots_a[..fired_a]);
                r.a_stats.add(na as u32);
                r.b_stats.add(nb as u32);
                let late = 2 * g >= match_len;
                if a_won {
                    r.a_wins += 1;
                    r.a_wins_late += late as u64;
                } else {
                    r.b_wins += 1;
                    r.b_wins_late += late as u64;
                }
                tally = (tally.0 + a_won as u32, tally.1 + 1);
            }
            r.matches.push(tally);
        },
        progress,
    );
    let mut total = DuelResult::default();
    for r in &per_thread {
        total.a_wins += r.a_wins;
        total.b_wins += r.b_wins;
        total.a_wins_late += r.a_wins_late;
        total.b_wins_late += r.b_wins_late;
        total.matches.extend(&r.matches);
        total.a_stats.merge(&r.a_stats);
        total.b_stats.merge(&r.b_stats);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{self, UniformPlacer};

    /// Shoots the board in reading order: every game takes the same shots.
    struct Sweep;

    impl Shooter for Sweep {
        fn name(&self) -> String {
            "sweep".into()
        }
        fn choose(&mut self, k: &Knowledge, _: &mut Rng) -> usize {
            k.unknown().0.trailing_zeros() as usize
        }
    }

    /// Records how many shots it was shown after each game.
    struct Watcher(std::sync::Arc<std::sync::Mutex<Vec<usize>>>);

    impl Placer for Watcher {
        fn name(&self) -> String {
            "watcher".into()
        }
        fn place(&mut self, _: &mut Rng) -> Layout {
            // Destroyer in the last two cells: a sweep needs all 100 shots.
            Layout::new([0, 20, 40, 60, 196]).unwrap()
        }
        fn learn(&mut self, _: &Layout, shots: &[usize]) {
            self.0.lock().unwrap().push(shots.len());
        }
    }

    #[test]
    fn duels_are_fair_to_identical_players() {
        // Identical deterministic players tie every game, so the first
        // mover wins it. Neither side may come out ahead, early or late.
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let player = Player {
            shooter: &|| Box::new(Sweep),
            placer: &|| Box::new(Watcher(seen.clone())),
        };
        let r = duel(&player, &player, 100, 2, 1, 2, &|_| {});
        assert_eq!((r.a_wins, r.b_wins), (50, 50));
        assert_eq!((r.a_wins_late, r.b_wins_late), (25, 25));
        // The loser never got to fire its hundredth shot.
        let count = |shots| seen.lock().unwrap().iter().filter(|&&n| n == shots).count();
        assert_eq!((count(100), count(99)), (100, 100));

        let half = duel(&player, &player, 1, 2, 1, 1, &|_| {});
        assert_eq!(half.a_rate_late(), None);
    }

    #[test]
    fn games_pair_up_across_shooters() {
        let run = |name: &'static str| {
            games(
                &|| ai::shooter(name).unwrap(),
                &|| Box::new(UniformPlacer),
                6,
                5,
                3,
                &|_| {},
            )
        };
        let (a, b) = (run("random"), run("parity"));
        assert!(a.iter().zip(&b).all(|(x, y)| x.layout == y.layout));
        assert!(a.iter().all(|g| (17..=100).contains(&g.shots)));
    }
}
