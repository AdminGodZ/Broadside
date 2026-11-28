//! Whether the opponent hides its fleet where we have looked late.
//!
//! An opponent who watches our shots learns more from a game than we do:
//! it sees the order of some 45 cells, we see 17 cells of its fleet. One
//! that moves its ships to where our hunting came late cannot be caught up
//! with by counting where its ships lay.
//!
//! But what it knows of us we know too: our own shots. So the engine keeps
//! the record such an opponent would keep and scores, on each fleet before
//! it is counted, views that expect ships where that record says a hider
//! would put them. A view is trusted by how well it has predicted, so
//! against an opponent who does not hide it stays out of the way.

use super::placement::hunt_times;
use super::repeats::SlotWeights;
use crate::bitboard::{Bits, CELLS};
use crate::fleet::{LENGTHS, SEGMENTS, SHIPS, SLOTS};
use crate::game::{Knowledge, Layout};
use crate::posterior::Posterior;
use std::sync::Arc;

/// What an opponent may do with the record of our hunting.
#[derive(Clone, Copy)]
enum View {
    /// Every cell is drawn toward late cells: a tilt of this strength per
    /// standard deviation of lateness.
    Cells(f64),
    /// A ship is drawn toward places where it would be found late, which is
    /// when we come to the earliest of its cells.
    Found(f64),
}

const VIEWS: [View; 5] = [
    View::Cells(0.4),
    View::Cells(0.8),
    View::Found(1.0),
    View::Found(2.0),
    View::Found(3.0),
];
/// Bounds on a view's score (ln of its likelihood against uniform
/// placement), so that an opponent who starts or stops hiding is followed
/// within a few games.
const SCORE: (f64, f64) = (-10.0, 12.0);
/// Below this share of belief the views are left out altogether.
const NEGLIGIBLE: f64 = 0.01;

#[derive(Clone)]
pub struct Hiding {
    /// Per cell: the summed time at which our hunting came to it (as
    /// `hunt_times` tells it), and the games behind the sums.
    late: [f64; CELLS],
    games: f64,
    scores: [f64; VIEWS.len()],
    /// What each view expects of the next fleet: ln of how much likelier
    /// than under uniform placement each ship lies at each slot.
    expect: Option<Arc<Vec<SlotWeights>>>,
    /// Belief, before any game, that the opponent hides from our hunting.
    pub prior: f64,
}

impl Default for Hiding {
    fn default() -> Self {
        Self::new()
    }
}

impl Hiding {
    pub const PRIOR: f64 = 0.01;

    pub fn new() -> Self {
        Self {
            late: [0.0; CELLS],
            games: 0.0,
            scores: [0.0; VIEWS.len()],
            expect: None,
            prior: Self::PRIOR,
        }
    }

    /// The chance of each slot per ship under uniform placement.
    fn uniform() -> Arc<SlotWeights> {
        static U: std::sync::OnceLock<Arc<SlotWeights>> = std::sync::OnceLock::new();
        U.get_or_init(|| {
            let p = Posterior::cached(&Knowledge::new(), 1);
            let mut u = [[0.0; SLOTS]; SHIPS];
            for (row, counts) in u.iter_mut().zip(p.ships.iter()) {
                for (x, &n) in row.iter_mut().zip(counts) {
                    *x = n as f64 / p.layouts as f64;
                }
            }
            Arc::new(u)
        })
        .clone()
    }

    /// What every view expects, from the record as it stands; `None` while
    /// the record says nothing (no game yet, or every cell alike).
    fn views(&self) -> Option<Arc<Vec<SlotWeights>>> {
        if self.games == 0.0 {
            return None;
        }
        let late = self.late.map(|l| l / self.games);
        let mu = late.iter().sum::<f64>() / CELLS as f64;
        let sd = (late.iter().map(|l| (l - mu).powi(2)).sum::<f64>() / CELLS as f64).sqrt();
        if sd < 1e-9 {
            return None;
        }
        let z = late.map(|l| (l - mu) / sd);
        let u = Self::uniform();
        let tables = VIEWS
            .iter()
            .map(|view| {
                let mut out = [[0.0; SLOTS]; SHIPS];
                for (s, row) in out.iter_mut().enumerate() {
                    let seg = &SEGMENTS[LENGTHS[s]];
                    let pull = |slot: usize| {
                        let cells = Bits(seg[slot]).map(|c| z[c]);
                        match *view {
                            View::Cells(b) => b * cells.sum::<f64>(),
                            View::Found(b) => b * cells.fold(f64::INFINITY, f64::min),
                        }
                    };
                    let legal = || (0..SLOTS).filter(|&slot| u[s][slot] > 0.0);
                    // ln of the mean weight under uniform placement, so that
                    // a weight says how much likelier than uniform a slot is.
                    let top = legal().map(pull).fold(f64::NEG_INFINITY, f64::max);
                    let mean = legal()
                        .map(|slot| u[s][slot] * (pull(slot) - top).exp())
                        .sum::<f64>()
                        .ln()
                        + top;
                    for slot in legal() {
                        row[slot] = pull(slot) - mean;
                    }
                }
                out
            })
            .collect();
        Some(Arc::new(tables))
    }

    /// Share of belief in each view, the rest being an opponent who does
    /// not hide.
    fn belief(&self) -> [f64; VIEWS.len()] {
        let mut w = [0.0; VIEWS.len()];
        for (w, s) in w.iter_mut().zip(self.scores) {
            *w = self.prior / VIEWS.len() as f64 * s.exp();
        }
        let z = 1.0 - self.prior + w.iter().sum::<f64>();
        w.map(|x| x / z)
    }

    /// The belief that the opponent hides from our hunting.
    pub fn trust(&self) -> f64 {
        self.belief().iter().sum()
    }

    /// A game is over: the opponent's fleet came to light, and it saw
    /// `shots`, ours at that fleet in order.
    pub fn observe(&mut self, layout: &Layout, shots: &[usize]) {
        if let Some(expect) = &self.expect {
            for (score, table) in self.scores.iter_mut().zip(expect.iter()) {
                let gain: f64 = (0..SHIPS).map(|s| table[s][layout.slots[s] as usize]).sum();
                *score = (*score + gain).clamp(SCORE.0, SCORE.1);
            }
        }
        for (l, t) in self.late.iter_mut().zip(hunt_times(layout, shots)) {
            *l += t;
        }
        self.games += 1.0;
        self.expect = self.views();
    }

    /// What is expected of the next fleet, as ln weights per ship and slot;
    /// `None` while that is uniform placement for all practical purposes.
    pub fn weights(&self) -> Option<Arc<SlotWeights>> {
        let belief = self.belief();
        let expect = self.expect.as_ref()?;
        if self.prior <= 0.0 || belief.iter().sum::<f64>() < NEGLIGIBLE {
            return None;
        }
        let rest = 1.0 - belief.iter().sum::<f64>();
        let mut out = [[0.0; SLOTS]; SHIPS];
        for (s, row) in out.iter_mut().enumerate() {
            for (slot, x) in row.iter_mut().enumerate() {
                if SEGMENTS[LENGTHS[s]][slot] != 0 {
                    let w: f64 = (0..VIEWS.len())
                        .map(|i| belief[i] * expect[i][s][slot].exp())
                        .sum();
                    *x = (rest + w).ln();
                }
            }
        }
        Some(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Placer;
    use crate::ai::engine::Broadside;
    use crate::ai::placement::AdaptivePlacer;
    use crate::rng::Rng;
    use crate::sim::play_game;

    /// Plays `games` games of the plain greedy engine against `placer`, the
    /// placer learning from the shots; returns what `Hiding` makes of it.
    fn watch(placer: &mut dyn Placer, games: usize, seed: u64) -> Hiding {
        let mut rng = Rng::new(seed);
        let mut h = Hiding::new();
        let mut shooter = Broadside::greedy();
        shooter.learner = None;
        for _ in 0..games {
            let layout = placer.place(&mut rng);
            let shots = play_game(&mut shooter, layout, &mut rng);
            placer.learn(&layout, &shots);
            h.observe(&layout, &shots);
        }
        h
    }

    #[test]
    fn a_hider_is_recognized() {
        let h = watch(&mut AdaptivePlacer::new(), 30, 1);
        assert!(h.trust() > 0.9, "{}", h.trust());
        assert!(h.weights().is_some());
    }

    #[test]
    fn random_fleets_leave_no_belief() {
        for seed in 0..5 {
            let h = watch(&mut crate::ai::UniformPlacer, 60, seed);
            assert!(h.weights().is_none(), "seed {seed}: trust {}", h.trust());
        }
    }

    #[test]
    fn weights_average_to_one_under_uniform_placement() {
        let h = watch(&mut AdaptivePlacer::new(), 5, 2);
        let u = Hiding::uniform();
        for table in h.expect.as_ref().unwrap().iter() {
            for s in 0..SHIPS {
                let mean: f64 = (0..SLOTS).map(|x| u[s][x] * table[s][x].exp()).sum();
                assert!((mean - 1.0).abs() < 1e-9, "ship {s}: {mean}");
            }
        }
    }
}
