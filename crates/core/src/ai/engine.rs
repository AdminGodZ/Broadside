//! The Broadside shooter: exact Bayesian inference over every fleet layout,
//! plus rollout search where it pays off.
//!
//! The base policy is greedy: fire at the cell most likely to hold a ship.
//! Greedy is myopic about misses that *reshape* the search, which matters most
//! when few ships remain and they hide in fragmented water. There we improve
//! on it by rollouts: for each candidate shot, play the greedy policy to the
//! end against layouts drawn from the exact posterior (the same draws for
//! every candidate, so comparisons are low-noise) and pick the candidate with
//! the fewest expected shots. One afloat ship is evaluated exactly, over every
//! placement it could have.

use super::Shooter;
use super::placement::Tilt;
use crate::bitboard::{Bits, CELLS};
use crate::fleet::{LENGTHS, SEGMENTS, SHIPS, Ship, THROUGH};
use crate::game::{Knowledge, Outcome};
use crate::posterior::Posterior;
use crate::rng::Rng;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Search {
    /// Rollouts run while at most this many ships are afloat (0 = never).
    pub max_afloat: usize,
    /// Most likely cells considered.
    pub candidates: usize,
    /// Sampled layouts per decision when the posterior is not enumerated.
    pub samples: usize,
    /// Enumerate the afloat ships' placements exactly when at most this many.
    pub exact_limit: usize,
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
                max_afloat: 2,
                candidates: 6,
                samples: 48,
                exact_limit: 400,
            },
            prior: None,
            learner: Some(Learner::default()),
        }
    }

    pub fn greedy() -> Self {
        Self {
            search: Search {
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

/// How much each cell is worth shooting: its probability of holding a ship.
///
/// Without a prior these are the exact layout counts. With one, each ship's
/// exact placement counts are reweighted by the prior and renormalized (a
/// product-of-experts approximation: exact constraints, approximate prior).
pub fn cell_scores(p: &Posterior, prior: Option<&Tilt>) -> [f64; CELLS] {
    let Some(t) = prior else {
        return p.cells.map(|c| c as f64);
    };
    let mut out = [0.0; CELLS];
    for (ship, &len) in LENGTHS.iter().enumerate() {
        let w = &p.ships[ship];
        let z: f64 = (0..w.len())
            .map(|slot| w[slot] as f64 * t.weight[ship][slot])
            .sum();
        if z == 0.0 {
            continue;
        }
        for slot in (0..w.len()).filter(|&slot| w[slot] > 0) {
            let v = w[slot] as f64 * t.weight[ship][slot] / z;
            for c in Bits(SEGMENTS[len][slot]) {
                out[c] += v;
            }
        }
    }
    out
}

/// Highest-scoring unshot cell, ties broken at random.
pub fn best_random(k: &Knowledge, scores: &[f64; CELLS], rng: &mut Rng) -> usize {
    let mut best = (f64::NEG_INFINITY, 0u128);
    for c in Bits(k.unknown().0) {
        if scores[c] > best.0 {
            best = (scores[c], 1 << c);
        } else if scores[c] == best.0 {
            best.1 |= 1 << c;
        }
    }
    rng.pick_bit(best.1)
}

/// Highest-scoring unshot cell, ties to the lowest cell. Rollouts need a
/// deterministic base policy: with random ties each evaluation is a noisy
/// draw, and picking the best of several noisy estimates selects noise.
pub fn best_det(k: &Knowledge, scores: &[f64; CELLS]) -> usize {
    let mut best = (f64::NEG_INFINITY, usize::MAX);
    for c in Bits(k.unknown().0) {
        if scores[c] > best.0 {
            best = (scores[c], c);
        }
    }
    best.1
}

/// Shots to finish the game from `k`, firing `first` and then greedily,
/// when the afloat ships truly lie at `truth` (slot per ship).
fn rollout(k: &Knowledge, first: usize, truth: &[Option<u8>; SHIPS], prior: Option<&Tilt>) -> u32 {
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
        cell = best_det(&k, &cell_scores(&Posterior::compute(&k), prior));
    }
}

impl Broadside {
    fn search(&self, k: &Knowledge, scores: &[f64; CELLS], rng: &mut Rng) -> usize {
        let prior = self.prior.as_deref();
        let unknown = k.unknown().0;
        let mut cands: Vec<usize> = Bits(unknown).collect();
        // Stable sort: ties keep the lowest cell first, matching `best_det`.
        cands.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
        cands.truncate(self.search.candidates);

        // The futures to evaluate against: (afloat placements, weight).
        // Enumerate them exactly when there are few enough, else sample.
        let enumerated = Posterior::enumerate_afloat(k, self.search.exact_limit);
        let exact = enumerated.is_some();
        let futures: Vec<([Option<u8>; SHIPS], f64)> = match enumerated {
            Some(f) => f.into_iter().map(|(t, w)| (t, w as f64)).collect(),
            None => (0..self.search.samples)
                .map(|_| (Posterior::sample_afloat(k, rng), 1.0))
                .collect(),
        };
        // Under a prior, each future also carries its placements' prior weight.
        let futures: Vec<_> = futures
            .into_iter()
            .map(|(t, w)| {
                let pw: f64 = prior.map_or(1.0, |pr| {
                    (0..SHIPS)
                        .filter_map(|s| t[s].map(|slot| pr.weight[s][slot as usize]))
                        .product()
                });
                (t, w * pw)
            })
            .collect();

        // cands[0] is the base policy's own move (most likely, lowest cell).
        let base: Vec<f64> = futures
            .iter()
            .map(|(t, _)| rollout(k, cands[0], t, prior) as f64)
            .collect();
        let total_w: f64 = futures.iter().map(|(_, w)| w).sum();
        let mut best = (0.0, cands[0]); // (expected shots saved vs base, cell)
        for &x in &cands[1..] {
            // Paired differences against the base move, same futures.
            let d: Vec<f64> = futures
                .iter()
                .zip(&base)
                .map(|((t, _), b)| b - rollout(k, x, t, prior) as f64)
                .collect();
            let mean = d.iter().zip(&futures).map(|(d, (_, w))| d * w).sum::<f64>() / total_w;
            // Sampled futures are noisy: only leave the base move for a
            // significant gain, or the search just chases sampling noise.
            let margin = if exact {
                1e-9
            } else {
                let n = d.len() as f64;
                let var = d.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
                2.0 * (var / n).sqrt() + 1e-9
            };
            if mean > margin && mean > best.0 {
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

    fn name(&self) -> String {
        if self.search.max_afloat == 0 {
            "greedy".into()
        } else {
            "broadside".into()
        }
    }

    fn choose(&mut self, k: &Knowledge, rng: &mut Rng) -> usize {
        let p = Posterior::cached(k, self.threads);
        let unknown = k.unknown().0;
        // A certain hit is never wrong to take first: it must be fired anyway.
        if let Some(c) = Bits(unknown).find(|&c| p.cells[c] == p.layouts) {
            return c;
        }
        let scores = cell_scores(&p, self.prior.as_deref());
        if (k.afloat_mask().count_ones() as usize) <= self.search.max_afloat {
            return self.search(k, &scores, rng);
        }
        best_random(k, &scores, rng)
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
