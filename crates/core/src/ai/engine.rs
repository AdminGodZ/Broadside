//! The Broadside shooter: exact Bayesian inference over every fleet layout,
//! plus search where it pays off.
//!
//! The base policy is greedy: fire at the cell most likely to hold a ship.
//! Greedy is myopic about what a shot teaches, which matters most when few
//! ships remain and they hide in fragmented water. Once the afloat ships have
//! few enough joint placements to list, the engine searches over all of them
//! (see `search`): small positions are solved outright, larger ones compare
//! the likeliest shots by playing each line to the end. Positions too large
//! to list can fall back on rollouts against sampled layouts.

use super::Shooter;
use super::placement::Tilt;
use super::search::{self, Tree};
use crate::bitboard::{Bits, CELLS};
use crate::fleet::{LENGTHS, SEGMENTS, SHIPS, Ship, THROUGH};
use crate::game::{Knowledge, Outcome};
use crate::posterior::Posterior;
use crate::rng::Rng;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Search {
    /// Search exactly while the afloat ships have at most this many joint
    /// placements (0 = never).
    pub exact_limit: usize,
    /// Most likely cells compared at each searched position.
    pub candidates: usize,
    /// With at most this many placements left, the position is solved for
    /// the optimal shot (0 = never, at most 64)...
    pub solve_worlds: usize,
    /// ...if the solver gets there by opening at most this many positions.
    pub solve_nodes: u64,
    /// Lines of play through positions with at most this many placements
    /// continue with the best of the likeliest cells instead of greedily.
    pub deep_worlds: usize,
    /// Positions too large to list: rollouts on sampled layouts while at most
    /// this many ships are afloat (0 = never).
    pub max_afloat: usize,
    /// Sampled layouts per decision.
    pub samples: usize,
}

pub struct Broadside {
    pub threads: usize,
    pub search: Search,
    /// Expected placement habits of the opponent (None = uniform).
    pub prior: Option<Arc<Tilt>>,
    /// Learns the opponent's habits from revealed layouts, replacing `prior`
    /// (None = keep `prior` fixed).
    pub learner: Option<Learner>,
}

/// Learns the opponent's placement habits by model selection.
///
/// Candidates: uniform placement, the tilt family, and free per-cell
/// occupancy shrunk toward uniform by various amounts. Before each revealed
/// layout is counted, every candidate predicts each cell's occupancy and is
/// scored by log-likelihood (prequential scoring), so a model only wins by
/// predicting unseen layouts better. Against a uniform placer, uniform wins and
/// nothing is lost to noise.
#[derive(Clone, Default)]
pub struct Learner {
    seen: Vec<u32>,
    games: u32,
    scores: Vec<f64>,
    targets: Vec<[f64; CELLS]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Model {
    Tilt(f64),
    PerCell(f64),
}

const MODELS: [Model; 15] = [
    Model::Tilt(0.0),
    Model::Tilt(0.25),
    Model::Tilt(0.5),
    Model::Tilt(0.75),
    Model::Tilt(1.0),
    Model::Tilt(1.25),
    Model::Tilt(1.5),
    Model::Tilt(1.75),
    Model::Tilt(2.0),
    Model::Tilt(2.5),
    Model::Tilt(3.0),
    Model::PerCell(10.0),
    Model::PerCell(30.0),
    Model::PerCell(100.0),
    Model::PerCell(300.0),
];

impl Learner {
    fn predict(&self, i: usize, u: &Posterior) -> [f64; CELLS] {
        match MODELS[i] {
            Model::Tilt(_) => self.targets[i],
            Model::PerCell(k) => {
                let n = self.games as f64;
                let mut out = [0.0; CELLS];
                for (c, o) in out.iter_mut().enumerate() {
                    *o = (self.seen[c] as f64 + k * u.prob(c)) / (n + k);
                }
                out
            }
        }
    }

    /// Scores every model on `layout`, then adds it; returns the best prior.
    fn observe(&mut self, layout: &crate::game::Layout) -> Option<Arc<Tilt>> {
        let u = Posterior::cached(&Knowledge::new(), 1);
        if self.seen.is_empty() {
            self.seen = vec![0; CELLS];
            self.scores = vec![0.0; MODELS.len()];
            self.targets = MODELS
                .iter()
                .map(|m| match *m {
                    Model::Tilt(a) => Tilt::target_occupancy(a),
                    Model::PerCell(_) => [0.0; CELLS],
                })
                .collect();
        }
        let occupied = layout.occupied();
        for i in 0..MODELS.len() {
            let p = self.predict(i, &u);
            self.scores[i] += (0..CELLS)
                .map(|c| {
                    let q = p[c].clamp(1e-3, 1.0 - 1e-3);
                    if occupied.has(c) {
                        q.ln()
                    } else {
                        (1.0 - q).ln()
                    }
                })
                .sum::<f64>();
        }
        for c in Bits(occupied.0) {
            self.seen[c] += 1;
        }
        self.games += 1;

        let best = (0..MODELS.len())
            .max_by(|&a, &b| self.scores[a].total_cmp(&self.scores[b]).then(b.cmp(&a)))?;
        match MODELS[best] {
            Model::Tilt(0.0) => None,
            Model::Tilt(a) => Some(super::placement::fitted(a)),
            Model::PerCell(_) => {
                // First-order tilt: occupancy scales with e^theta.
                let p = self.predict(best, &u);
                let mut theta = [0.0; CELLS];
                for (c, t) in theta.iter_mut().enumerate() {
                    *t = (p[c] / u.prob(c)).ln();
                }
                Some(Arc::new(Tilt::from_theta(theta)))
            }
        }
    }
}

impl Broadside {
    pub fn new() -> Self {
        Self {
            threads: 1,
            search: Search {
                exact_limit: 200_000,
                candidates: 6,
                solve_worlds: 16,
                solve_nodes: 2_000,
                deep_worlds: 0,
                max_afloat: 0,
                samples: 48,
            },
            prior: None,
            learner: Some(Learner::default()),
        }
    }

    /// No search: always the likeliest cell.
    pub fn greedy() -> Self {
        Self {
            search: Search {
                exact_limit: 0,
                max_afloat: 0,
                ..Self::new().search
            },
            ..Self::new()
        }
    }
}

impl Default for Broadside {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcomes other than a miss that a shot at `cell` could produce.
pub fn occupied_outcomes(k: &Knowledge, cell: usize) -> Vec<Outcome> {
    let mut out = vec![Outcome::Hit];
    for s in (0..SHIPS).filter(|&s| k.sunk[s].is_none()) {
        let len = LENGTHS[s];
        let t = THROUGH[len][cell];
        let can_finish = t.slots[..t.n as usize]
            .iter()
            .any(|&slot| SEGMENTS[len][slot as usize] & !(k.hits.0 | 1 << cell) == 0);
        if can_finish {
            out.push(Outcome::Sunk(Ship::from_index(s)));
        }
    }
    out
}

/// How much each unshot cell is worth shooting: its probability of holding
/// a ship.
///
/// Without a prior these are the exact layout counts. With one, each afloat
/// ship's exact placement counts are reweighted by the prior and renormalized
/// (a product-of-experts approximation: exact constraints, approximate
/// prior). Only a placement's unshot cells count: every consistent layout
/// holds all the hit cells, so their weight says nothing.
pub fn cell_scores(p: &Posterior, prior: Option<&Tilt>, k: &Knowledge) -> [f64; CELLS] {
    let Some(t) = prior else {
        return p.cells.map(|c| c as f64);
    };
    let unknown = k.unknown().0;
    let mut out = [0.0; CELLS];
    for ship in Bits(k.afloat_mask() as u128) {
        let (len, w) = (LENGTHS[ship], &p.ships[ship]);
        let log = |slot: usize| -> f64 {
            Bits(SEGMENTS[len][slot] & unknown)
                .map(|c| t.theta[c])
                .sum()
        };
        let live = || (0..w.len()).filter(|&slot| w[slot] > 0);
        let top = live().map(log).fold(f64::NEG_INFINITY, f64::max);
        let z: f64 = live()
            .map(|slot| w[slot] as f64 * (log(slot) - top).exp())
            .sum();
        for slot in live() {
            let v = w[slot] as f64 * (log(slot) - top).exp() / z;
            for c in Bits(SEGMENTS[len][slot] & unknown) {
                out[c] += v;
            }
        }
    }
    out
}

/// Highest-scoring cell of `pool`, ties broken at random.
pub fn best_random(pool: u128, scores: &[f64; CELLS], rng: &mut Rng) -> usize {
    let mut best = (f64::NEG_INFINITY, 0u128);
    for c in Bits(pool) {
        if scores[c] > best.0 {
            best = (scores[c], 1 << c);
        } else if scores[c] == best.0 {
            best.1 |= 1 << c;
        }
    }
    rng.pick_bit(best.1)
}

/// Highest-scoring cell of `pool`, ties to the lowest cell. Rollouts need a
/// deterministic base policy: with random ties each evaluation is a noisy
/// draw, and picking the best of several noisy estimates selects noise.
pub fn best_det(pool: u128, scores: &[f64; CELLS]) -> usize {
    let mut best = (f64::NEG_INFINITY, usize::MAX);
    for c in Bits(pool) {
        if scores[c] > best.0 {
            best = (scores[c], c);
        }
    }
    best.1
}

/// Unshot cells that hold a ship in every consistent layout, and the lowest
/// one that the same ship holds in every layout.
///
/// The second kind is never wrong to fire at first: it must be hit anyway, and
/// hitting it early can only bring that ship's sinking report forward. A cell
/// that is certain but could belong to different ships is another matter: when
/// it is hit decides which later shot announces which sinking, and that can
/// cost information.
fn certain_cells(p: &Posterior, k: &Knowledge) -> (u128, Option<usize>) {
    let certain = Bits(k.unknown().0)
        .filter(|&c| p.cells[c] == p.layouts)
        .fold(0u128, |m, c| m | 1 << c);
    if certain == 0 {
        return (0, None);
    }
    let by_ship = p.ship_cells();
    let forced = Bits(certain).find(|&c| by_ship.iter().any(|s| s[c] == p.layouts));
    (certain, forced)
}

/// The base policy's shot: a cell one ship certainly holds, else a certain
/// hit, else the likeliest cell; lowest cell on ties.
fn base_move(k: &Knowledge, p: &Posterior, prior: Option<&Tilt>) -> usize {
    let (certain, forced) = certain_cells(p, k);
    if let Some(c) = forced {
        return c;
    }
    let pool = if certain != 0 { certain } else { k.unknown().0 };
    best_det(pool, &cell_scores(p, prior, k))
}

/// The base policy's shot per position. Rollouts from one decision keep
/// meeting the same positions: before their layouts tell them apart, and
/// where two lines fire the same shots in a different order.
pub(crate) type BaseMoves = HashMap<Knowledge, usize>;

/// Shots to finish the game from `k`, firing `first` and then following the
/// base policy, when the afloat ships truly lie at `truth` (slot per ship).
pub(crate) fn rollout(
    k: &Knowledge,
    first: usize,
    truth: &[Option<u8>; SHIPS],
    prior: Option<&Tilt>,
    seen: &mut BaseMoves,
) -> u32 {
    let ships: Vec<(usize, u128)> = (0..SHIPS)
        .filter_map(|s| truth[s].map(|slot| (s, SEGMENTS[LENGTHS[s]][slot as usize])))
        .collect();
    let mut k = *k;
    let mut cell = first;
    let mut shots = 0;
    loop {
        let outcome = match ships.iter().find(|(_, m)| m >> cell & 1 == 1) {
            None => Outcome::Miss,
            Some(&(s, m)) if m & !(k.hits.0 | 1 << cell) == 0 => Outcome::Sunk(Ship::from_index(s)),
            Some(_) => Outcome::Hit,
        };
        k.record(cell, outcome);
        shots += 1;
        if k.won() {
            return shots;
        }
        cell = *seen
            .entry(k)
            .or_insert_with(|| base_move(&k, &Posterior::cached(&k, 1), prior));
    }
}

impl Broadside {
    /// Rollout search against layouts sampled from the posterior, for
    /// positions with too many placements to list. `cands`: cells to compare,
    /// the base policy's own move first.
    fn sampled_search(
        &self,
        k: &Knowledge,
        p: &Posterior,
        cands: &[usize],
        rng: &mut Rng,
    ) -> usize {
        let prior = self.prior.as_deref();
        let unknown = k.unknown().0;
        let futures = p.sample_afloat(k, self.search.samples, rng);
        // The samples follow the uniform posterior. Under a prior each
        // counts by its prior weight, which only its unshot cells set apart.
        let logs: Vec<f64> = futures
            .iter()
            .map(|t| {
                let all = (0..SHIPS)
                    .filter_map(|s| t[s].map(|slot| SEGMENTS[LENGTHS[s]][slot as usize]))
                    .fold(0u128, |a, m| a | m);
                prior.map_or(0.0, |pr| Bits(all & unknown).map(|c| pr.theta[c]).sum())
            })
            .collect();
        let top = logs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let w: Vec<f64> = logs.iter().map(|l| (l - top).exp()).collect();
        let (n, total) = (w.len() as f64, w.iter().sum::<f64>());

        let mut seen = BaseMoves::new();
        let base: Vec<f64> = futures
            .iter()
            .map(|t| rollout(k, cands[0], t, prior, &mut seen) as f64)
            .collect();
        let mut best = (0.0, cands[0]); // (expected shots saved vs base, cell)
        for &x in &cands[1..] {
            // Paired differences against the base move, same futures.
            let d: Vec<f64> = futures
                .iter()
                .zip(&base)
                .map(|(t, b)| b - rollout(k, x, t, prior, &mut seen) as f64)
                .collect();
            let mean = d.iter().zip(&w).map(|(d, w)| d * w).sum::<f64>() / total;
            // Sampled futures are noisy: only leave the base move for a
            // significant gain, or the search just chases sampling noise.
            // (Standard error of a weighted mean; s/sqrt(n) for equal weights.)
            let var = n / (n - 1.0)
                * d.iter()
                    .zip(&w)
                    .map(|(d, w)| (w * (d - mean)).powi(2))
                    .sum::<f64>()
                / (total * total);
            if mean > 2.0 * var.sqrt() + 1e-9 && mean > best.0 {
                best = (mean, x);
            }
        }
        best.1
    }
}

impl Shooter for Broadside {
    fn learn(&mut self, layout: &crate::game::Layout) {
        if let Some(l) = self.learner.as_mut() {
            self.prior = l.observe(layout);
        }
    }

    fn set_threads(&mut self, threads: usize) {
        self.threads = threads.max(1);
    }

    fn name(&self) -> String {
        if self.search.exact_limit == 0 && self.search.max_afloat == 0 {
            "greedy".into()
        } else {
            "broadside".into()
        }
    }

    fn choose(&mut self, k: &Knowledge, rng: &mut Rng) -> usize {
        let p = Posterior::cached(k, self.threads);
        let unknown = k.unknown().0;
        if p.layouts == 0 {
            // Contradictory reports: nothing to reason from.
            return rng.pick_bit(unknown);
        }
        let (certain, forced) = certain_cells(&p, k);
        if let Some(c) = forced {
            return c;
        }
        let prior = self.prior.as_deref();
        let s = &self.search;
        if let Some(worlds) = search::worlds(k, &p, s.exact_limit, prior) {
            return Tree::new(&worlds, *s).choose(k.hits.0, self.threads);
        }
        let scores = cell_scores(&p, prior, k);
        // Greedy takes a certain hit before anything else.
        let pool = if certain != 0 { certain } else { unknown };
        if (k.afloat_mask().count_ones() as usize) <= s.max_afloat && s.samples >= 2 {
            let mut cands: Vec<usize> = Bits(pool).collect();
            // Stable sort: ties keep the lowest cell first, like `best_det`.
            cands.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
            cands.truncate(s.candidates.max(1));
            if cands.len() == 1 {
                return cands[0];
            }
            // Its own random stream: how many samples it draws must not
            // shift the tie-breaks of later moves.
            return self.sampled_search(k, &p, &cands, &mut Rng::new(rng.next_u64()));
        }
        best_random(pool, &scores, rng)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Board, Layout};

    #[test]
    fn outcome_posteriors_partition_occupied_layouts() {
        let mut rng = Rng::new(3);
        for _ in 0..40 {
            let layout = Layout::random(&mut rng);
            let mut board = Board::new(layout);
            let mut k = Knowledge::new();
            let shots = 50 + rng.below(30);
            for _ in 0..shots {
                if k.won() {
                    break;
                }
                let h = k.hits.0;
                let near = crate::bitboard::neighbours4(h) & k.unknown().0;
                let c = if near != 0 {
                    rng.pick_bit(near)
                } else {
                    rng.pick_bit(k.unknown().0)
                };
                let o = board.fire(c);
                k.record(c, o);
            }
            if k.won() {
                continue;
            }
            let p = Posterior::compute(&k);
            for x in Bits(k.unknown().0) {
                let total: u64 = occupied_outcomes(&k, x)
                    .into_iter()
                    .map(|o| {
                        let mut k2 = k;
                        k2.record(x, o);
                        Posterior::compute(&k2).layouts
                    })
                    .sum();
                assert_eq!(total, p.cells[x], "cell {x} in {k:?}");
            }
        }
    }
}

#[cfg(test)]
mod learner_tests {
    use super::*;
    use crate::game::Layout;

    #[test]
    fn learner_keeps_uniform_prior_against_uniform_placement() {
        let mut l = Learner::default();
        let mut rng = Rng::new(17);
        let mut prior = None;
        for _ in 0..80 {
            prior = l.observe(&Layout::random(&mut rng));
        }
        assert!(
            prior.is_none(),
            "picked a non-uniform model against uniform layouts"
        );
    }

    #[test]
    fn learner_recognizes_edge_heavy_placement() {
        let mut l = Learner::default();
        let mut placer = super::super::placement::TiltedPlacer::new(2.0);
        let mut rng = Rng::new(18);
        let mut prior = None;
        for _ in 0..40 {
            prior = l.observe(&super::super::Placer::place(&mut placer, &mut rng));
        }
        let corner = prior.expect("a non-uniform model").theta[0];
        assert!(corner > 1.0, "corner weight {corner}");
    }
}
