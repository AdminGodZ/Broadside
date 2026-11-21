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
use super::habits::{self, Habits, KINDS, Mix};
use super::last;
use super::placement::Tilt;
use super::repeats::{Repeats, SlotWeights};
use super::search::{self, Tree};
use crate::bitboard::{Bits, CELLS};
use crate::fleet::{FLEET_CELLS, LENGTHS, SEGMENTS, SHIPS, Ship, THROUGH};
use crate::game::{Knowledge, Layout, Outcome};
use crate::posterior::Posterior;
use crate::rng::Rng;
use std::sync::Arc;

/// What the shooter expects of the opponent's placement beyond the rules:
/// habits per cell, habits per ship and placement, or both.
#[derive(Clone, Copy, Default)]
pub struct Prior<'a> {
    /// P(layout) ∝ Π e^θ over its cells.
    pub tilt: Option<&'a Tilt>,
    /// ln weight of each ship at each slot.
    pub slots: Option<&'a SlotWeights>,
}

impl Prior<'_> {
    pub const NONE: Prior<'static> = Prior {
        tilt: None,
        slots: None,
    };

    pub fn is_none(&self) -> bool {
        self.tilt.is_none() && self.slots.is_none()
    }

    /// ln weight of `ship` at `slot` when `unknown` are the unshot cells.
    /// Of the tilt only the unshot cells count: every consistent layout
    /// holds all the hit cells, so their weight says nothing.
    #[inline]
    pub fn ln_weight(&self, ship: usize, slot: usize, unknown: u128) -> f64 {
        let cells = self.tilt.map_or(0.0, |t| {
            Bits(SEGMENTS[LENGTHS[ship]][slot] & unknown)
                .map(|c| t.theta[c])
                .sum()
        });
        cells + self.slots.map_or(0.0, |w| w[ship][slot])
    }
}

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
    /// Positions too large to list: search over sampled placements while at
    /// most this many ships are afloat (0 = never).
    pub max_afloat: usize,
    /// Placements sampled per decision.
    pub samples: usize,
    /// The last ship, while none of its cells is hit: the best order to look
    /// for it is searched exactly, opening at most this many positions
    /// (0 = never; see `last`).
    pub last_nodes: u64,
    /// Against an opponent: rollouts of its play that estimate how many
    /// shots it still needs; the search then plays for the win, not for the
    /// fewest shots (0 = ignore the race).
    pub race: usize,
    /// How far, in tenths of a shot, the opponent may stray from the
    /// rollouts' estimate.
    pub spread: usize,
    /// While hunting with the Destroyer afloat, a cell of a smallest cover
    /// of its places counts this much more: its chance of a ship is
    /// multiplied by one plus this (0 = no cover rule; see `cover`).
    pub cover: f64,
    /// Lines of play in the search follow the cover rule in positions of at
    /// least this many placements (0 = never).
    pub rollout_cover: usize,
}

pub struct Broadside {
    pub threads: usize,
    pub search: Search,
    /// Expected placement habits of the opponent (None = uniform).
    pub prior: Option<Arc<Tilt>>,
    /// Learns the opponent's habits from revealed layouts, replacing `prior`
    /// (None = keep `prior` fixed).
    pub learner: Option<Learner>,
    /// In a game against an opponent: our fleet and what it knows of it.
    pub race: Option<(Layout, Knowledge)>,
    /// What the opponent is believed to do about spacing its ships; learns
    /// from its fleets while `learner` is set.
    pub habits: Habits,
    /// Whether the opponent puts ships back where they lay before; learns
    /// from its fleets while `learner` is set.
    pub repeats: Repeats,
    /// EXPERIMENT: weight of a ship's chance on a cell, by ship.
    pub hunt: [f64; SHIPS],
    /// EXPERIMENT: the cover bonus while a hit is unexplained (0 = none).
    pub cover_target: f64,
    /// EXPERIMENT: the cover rule also holds under a learned belief.
    pub cover_belief: bool,
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
                samples: 4000,
                last_nodes: 2_000,
                race: 0,
                spread: 15,
                cover: 0.0,
                rollout_cover: 0,
            },
            prior: None,
            learner: Some(Learner::default()),
            race: None,
            habits: Habits::neutral(),
            repeats: Repeats::new(),
            hunt: [1.0; SHIPS],
            cover_target: 0.0,
            cover_belief: false,
        }
    }

    /// The chance of a ship on each unshot cell as the engine sees it, with
    /// everything it has learned about the opponent (0 for shot cells).
    pub fn chances(&self, k: &Knowledge) -> [f64; CELLS] {
        let p = Posterior::cached(k, self.threads);
        let slots = self.repeats.weights();
        let prior = Prior {
            tilt: self.prior.as_deref(),
            slots: slots.as_deref(),
        };
        let mix = self
            .spacing()
            .and_then(|kinds| Mix::new(k, &p, &kinds, self.threads));
        // Scores are chances under a prior, weights of layouts without.
        let (scores, total) = match (&mix, prior.is_none()) {
            (Some(m), _) => (m.scores(prior, k), m.weight()),
            (None, false) => (cell_scores(&p, prior, k), 1.0),
            (None, true) => (cell_scores(&p, prior, k), p.layouts as f64),
        };
        let unknown = k.unknown();
        let mut out = [0.0; CELLS];
        for c in (0..CELLS).filter(|&c| unknown.has(c) && total > 0.0) {
            out[c] = (scores[c] / total).min(1.0);
        }
        out
    }

    /// The belief about the opponent's spacing now; `None` while it is what
    /// the rules alone say.
    pub fn spacing(&self) -> Option<[f64; KINDS]> {
        let kinds = self.habits.belief();
        (kinds != habits::neutral()).then_some(kinds)
    }

    /// No search: always the likeliest cell.
    pub fn greedy() -> Self {
        Self {
            search: Search {
                exact_limit: 0,
                max_afloat: 0,
                last_nodes: 0,
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
pub fn cell_scores(p: &Posterior, prior: Prior, k: &Knowledge) -> [f64; CELLS] {
    if prior.is_none() {
        return p.cells.map(|c| c as f64);
    }
    let unknown = k.unknown().0;
    let mut out = [0.0; CELLS];
    for ship in Bits(k.afloat_mask() as u128) {
        let (len, w) = (LENGTHS[ship], &p.ships[ship]);
        let log = |slot: usize| prior.ln_weight(ship, slot, unknown);
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

#[cfg(test)]
/// The base policy's shot: a cell one ship certainly holds, else a certain
/// hit, else the likeliest cell; lowest cell on ties.
fn base_move(k: &Knowledge, p: &Posterior, prior: Prior) -> usize {
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
#[cfg(test)]
pub(crate) type BaseMoves = std::collections::HashMap<Knowledge, usize>;

#[cfg(test)]
/// Shots to finish the game from `k`, firing `first` and then following the
/// base policy, when the afloat ships truly lie at `truth` (slot per ship).
pub(crate) fn rollout(
    k: &Knowledge,
    first: usize,
    truth: &[Option<u8>; SHIPS],
    prior: Prior,
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

/// Shots the opponent still needs from what it knows, `k`, against our
/// fleet, playing greedily (random ties).
fn opponent_rollout(fleet: &Layout, k: &Knowledge, rng: &mut Rng) -> usize {
    let mut k = *k;
    let mut shots = 0;
    while !k.won() {
        let p = Posterior::cached(&k, 1);
        let (certain, forced) = certain_cells(&p, &k);
        let pool = if certain != 0 { certain } else { k.unknown().0 };
        let cell =
            forced.unwrap_or_else(|| best_random(pool, &cell_scores(&p, Prior::NONE, &k), rng));
        let outcome = match fleet.ship_at(cell) {
            None => Outcome::Miss,
            Some(s) if fleet.ship(s).0 & !(k.hits.0 | 1 << cell) == 0 => Outcome::Sunk(s),
            Some(_) => Outcome::Hit,
        };
        k.record(cell, outcome);
        shots += 1;
    }
    shots
}

impl Broadside {
    /// `lose[t]`: the chance that the opponent sinks us first if we need
    /// `t` more shots, this one included; `None` unless that depends on how
    /// we play from here.
    fn race_odds(&self, k: &Knowledge, rng: &mut Rng) -> Option<Vec<f64>> {
        let (fleet, against) = self.race.as_ref()?;
        let n = self.search.race;
        if n == 0 {
            return None;
        }
        // P(U = u) for the opponent's remaining shots U, each rollout blurred
        // over the counts it can still reach.
        let floor = 17 - against.hits.count() as usize;
        let mut dist = vec![0.0; CELLS + 2];
        let sigma = self.search.spread as f64 / 10.0;
        for _ in 0..n {
            let u = opponent_rollout(fleet, against, rng);
            if sigma == 0.0 || u == 0 {
                dist[u] += 1.0;
                continue;
            }
            let w: Vec<f64> = (floor..=CELLS)
                .map(|v| (-((v as f64 - u as f64) / sigma).powi(2) / 2.0).exp())
                .collect();
            let z: f64 = w.iter().sum();
            for (v, w) in (floor..=CELLS).zip(w) {
                dist[v] += w / z;
            }
        }
        let mut lose = vec![0.0; CELLS + 2];
        for t in 1..lose.len() {
            lose[t] = lose[t - 1] + dist[t - 1] / n as f64;
        }
        // Between the fewest shots we could need and the most.
        let (least, most) = (17 - k.hits.count() as usize, k.unknown().count() as usize);
        let live = lose[least] < 1.0 - 1e-3 && lose[most] > 1e-3;
        // Fewer shots still break ties: they win against any opponent.
        live.then(|| {
            lose.iter()
                .enumerate()
                .map(|(t, l)| l + 1e-3 * t as f64)
                .collect()
        })
    }
}

impl Shooter for Broadside {
    fn learn(&mut self, layout: &crate::game::Layout) {
        if let Some(l) = self.learner.as_mut() {
            self.prior = l.observe(layout);
            self.habits.observe(layout.contact());
            self.repeats.observe(layout);
        }
    }

    fn set_threads(&mut self, threads: usize) {
        self.threads = threads.max(1);
    }

    fn race(&mut self, fleet: &Layout, against: &Knowledge) {
        self.race = Some((*fleet, *against));
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
        // A belief about spacing that fits nothing known was wrong: the
        // rules alone are left.
        let mix = self
            .spacing()
            .and_then(|kinds| Mix::new(k, &p, &kinds, self.threads));
        let (certain, forced) = certain_cells(mix.as_ref().map_or(&p, |m| &m.support), k);
        if let Some(c) = forced {
            return c;
        }
        let slots = self.repeats.weights();
        let prior = Prior {
            tilt: self.prior.as_deref(),
            slots: slots.as_deref(),
        };
        let s = self.search;
        // One ship left and no hit on it: every placement in the unshot
        // cells is as likely as any other, and the best order to try them
        // can be found outright.
        let afloat = k.afloat_mask();
        let plain = mix.is_none() && prior.is_none() && (self.race.is_none() || s.race == 0);
        if s.last_nodes > 0 && plain && afloat.count_ones() == 1 {
            let len = LENGTHS[afloat.trailing_zeros() as usize];
            if k.hits.count() as usize + len == FLEET_CELLS as usize
                && let Some(order) = last::best(k.unknown(), len, s.last_nodes)
            {
                return order.shot;
            }
        }
        let worlds = match &mix {
            Some(m) => m.worlds(k, s.exact_limit, prior),
            None => search::worlds(k, &p, s.exact_limit, prior),
        };
        if let Some(worlds) = worlds {
            let mut tree = Tree::new(&worlds, s);
            // Its own random stream (as below): whatever the search draws
            // must not shift the tie-breaks of later moves.
            let race = (self.race.is_some() && s.race > 0)
                .then(|| self.race_odds(k, &mut Rng::new(rng.next_u64())))
                .flatten();
            return match race {
                Some(lose) => tree.choose_for(k.hits.0, self.threads, &lose),
                None => tree.choose(k.hits.0, self.threads),
            };
        }
        let mut scores = match &mix {
            Some(m) => m.scores(prior, k),
            None => cell_scores(&p, prior, k),
        };
        let sunk_len: u32 = (0..SHIPS)
            .filter(|&s| k.sunk[s].is_some())
            .map(|s| LENGTHS[s] as u32)
            .sum();
        let hunting = k.hits.count() == sunk_len;
        let believing = mix.is_some() || !prior.is_none();
        if !believing && self.hunt != [1.0; SHIPS] {
            let by_ship = p.ship_cells();
            scores = [0.0; CELLS];
            for s in Bits(k.afloat_mask() as u128) {
                for c in 0..CELLS {
                    scores[c] += self.hunt[s] * by_ship[s][c] as f64;
                }
            }
        }
        // Greedy takes a certain hit before anything else.
        let pool = if certain != 0 { certain } else { unknown };
        // Hunting the Destroyer is a covering problem (see `cover`): cells
        // of a smallest cover of its places count for more.
        let bonus = if hunting { s.cover } else { self.cover_target };
        if bonus > 0.0 && certain == 0 && k.sunk[4].is_none() && (self.cover_belief || !believing) {
            let live = &mix.as_ref().map_or(&p, |m| &m.support).ships[4];
            let (mut across, mut down) = (0u128, 0u128);
            for c in Bits(unknown) {
                if live[2 * c] > 0 && unknown >> (c + 1) & 1 == 1 {
                    across |= 1 << c;
                }
                if live[2 * c + 1] > 0 && unknown >> (c + 10) & 1 == 1 {
                    down |= 1 << c;
                }
            }
            for c in Bits(super::cover::cover_cells(across, down)) {
                scores[c] *= 1.0 + bonus;
            }
        }
        let sample = mix.is_none() && s.samples >= 2;
        if (k.afloat_mask().count_ones() as usize) <= s.max_afloat && sample {
            let mut cands: Vec<usize> = Bits(pool).collect();
            // Stable sort: ties keep the lowest cell first, like `best_det`.
            cands.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
            cands.truncate(s.candidates.max(1));
            if cands.len() > 1 {
                let mut own = Rng::new(rng.next_u64());
                let worlds = search::sampled(k, &p, s.samples, prior, &mut own);
                return Tree::new(&worlds, s).choose_sampled(k.hits.0, self.threads, &cands);
            }
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
