//! Tilted placement: layouts weighted by per-cell factors, tuned so the
//! probability that each cell holds a ship hits a chosen target.
//!
//! P(layout) ∝ Π over ship cells of e^θ[cell], restricted to legal layouts.
//! The target occupancy is `u^(1-α)` (normalized to 17 ships cells), where `u`
//! is the occupancy under uniform placement: α = 0 is uniform, α = 1 makes every
//! cell equally likely, α > 1 inverts the centre bias. Shooters that assume
//! uniform placement hunt the centre first, so mass moved outward costs them.

use super::Placer;
use crate::bitboard::{Bits, CELLS};
use crate::fleet::{FLEET_CELLS, LENGTHS, SEGMENTS, SHIPS, SLOTS};
use crate::game::{Knowledge, Layout};
use crate::posterior::Posterior;
use crate::rng::Rng;
use crate::symmetry::CELL_MAP;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// A placement distribution: P(layout) ∝ Π e^θ[cell] over its ship cells.
#[derive(Clone)]
pub struct Tilt {
    pub theta: [f64; CELLS],
    /// Relative weight of each placement, by ship and slot (0 off the board),
    /// scaled so each ship's legal slots average 1. Adding a constant to
    /// every θ leaves the distribution unchanged, and these weights too.
    pub weight: [[f64; SLOTS]; SHIPS],
    cdf: [[f64; SLOTS]; SHIPS],
}

/// Whole-fleet draws rejected for overlap before `Tilt::sample` switches to
/// placing ship by ship (only extreme tilts get there).
const MAX_REJECTS: usize = 10_000;

impl Tilt {
    /// Panics on a θ that is not finite.
    pub fn from_theta(theta: [f64; CELLS]) -> Self {
        assert!(theta.iter().all(|t| t.is_finite()), "tilt must be finite");
        let mut weight = [[0.0; SLOTS]; SHIPS];
        let mut cdf = [[0.0; SLOTS]; SHIPS];
        for s in 0..SHIPS {
            let seg = &SEGMENTS[LENGTHS[s]];
            let log = |slot: usize| Bits(seg[slot]).map(|x| theta[x]).sum::<f64>();
            let legal = || (0..SLOTS).filter(|&slot| seg[slot] != 0);
            // Relative to the heaviest slot, so nothing overflows.
            let top = legal().map(log).fold(f64::NEG_INFINITY, f64::max);
            let mean =
                legal().map(|slot| (log(slot) - top).exp()).sum::<f64>() / legal().count() as f64;
            let mut acc = 0.0;
            for slot in 0..SLOTS {
                if seg[slot] != 0 {
                    weight[s][slot] = (log(slot) - top).exp() / mean;
                }
                acc += weight[s][slot];
                cdf[s][slot] = acc;
            }
        }
        Self { theta, weight, cdf }
    }

    pub fn sample(&self, rng: &mut Rng) -> Layout {
        for _ in 0..MAX_REJECTS {
            let mut slots = [0u8; SHIPS];
            let mut used = 0u128;
            let mut ok = true;
            for (s, slot) in slots.iter_mut().enumerate() {
                let cdf = &self.cdf[s];
                let r = rng.f64() * cdf[SLOTS - 1];
                // The first slot whose cumulative weight passes r: never an
                // off-board slot, those add no weight.
                let k = cdf.partition_point(|&c| c <= r).min(SLOTS - 1);
                *slot = k as u8;
                let m = SEGMENTS[LENGTHS[s]][k];
                ok &= m != 0 && m & used == 0;
                used |= m;
            }
            if ok {
                return Layout { slots };
            }
        }
        // Nearly every draw overlaps (the tilt piles all ships onto the same
        // cells): place ship by ship among the slots still free instead.
        loop {
            let mut slots = [0u8; SHIPS];
            let mut used = 0u128;
            let placed = slots.iter_mut().enumerate().all(|(s, slot)| {
                let seg = &SEGMENTS[LENGTHS[s]];
                let free = |k: usize| seg[k] != 0 && seg[k] & used == 0;
                let total: f64 = (0..SLOTS)
                    .filter(|&k| free(k))
                    .map(|k| self.weight[s][k])
                    .sum();
                let mut r = rng.f64() * total;
                let pick = (0..SLOTS).filter(|&k| free(k)).find(|&k| {
                    r -= self.weight[s][k];
                    r < 0.0
                });
                let Some(k) = pick.or_else(|| (0..SLOTS).find(|&k| free(k))) else {
                    return false;
                };
                *slot = k as u8;
                used |= seg[k];
                true
            });
            if placed {
                return Layout { slots };
            }
        }
    }

    /// Target occupancy of tilt `alpha`: `u^(1-alpha)`, scaled to 17 ship cells.
    pub fn target_occupancy(alpha: f64) -> [f64; CELLS] {
        assert!(alpha.is_finite(), "tilt alpha must be finite");
        let p = Posterior::cached(&Knowledge::new(), 1);
        let raw: Vec<f64> = (0..CELLS).map(|c| p.prob(c).powf(1.0 - alpha)).collect();
        let scale = FLEET_CELLS as f64 / raw.iter().sum::<f64>();
        let mut out = [0.0; CELLS];
        for (o, r) in out.iter_mut().zip(raw) {
            *o = (r * scale).min(0.999);
        }
        out
    }

    /// Fits θ by iterative proportional scaling on sampled occupancies.
    pub fn fit(alpha: f64, rounds: usize, samples: usize, seed: u64) -> Self {
        let target = Self::target_occupancy(alpha);

        let mut rng = Rng::new(seed);
        let mut tilt = Self::from_theta([0.0; CELLS]);
        for round in 0..rounds {
            let mut occ = [0u64; CELLS];
            for _ in 0..samples {
                for c in Bits(tilt.sample(&mut rng).occupied().0) {
                    occ[c] += 1;
                }
            }
            // Damped steps: raising one cell's weight also fills its
            // neighbours (ships are long), so full steps overshoot and oscillate.
            let step = if round < rounds / 2 { 0.5 } else { 0.25 };
            let mut theta = tilt.theta;
            for c in 0..CELLS {
                let m = (occ[c] as f64 + 0.5) / samples as f64;
                theta[c] += step * (target[c] / m).ln();
            }
            // The target is symmetric, so the answer is: averaging over the
            // board's symmetries cancels sampling noise and asymmetric drift.
            let mut sym = [0.0; CELLS];
            for (c, v) in sym.iter_mut().enumerate() {
                *v = CELL_MAP.iter().map(|m| theta[m[c] as usize]).sum::<f64>()
                    / CELL_MAP.len() as f64;
            }
            tilt = Self::from_theta(sym);
        }
        tilt
    }
}

/// The tilt for `alpha`: precomputed (`broadside fit-tilts`) when available,
/// else fitted on first use. Shared between placers and shooters.
pub fn fitted(alpha: f64) -> Arc<Tilt> {
    static FITS: OnceLock<Mutex<HashMap<u64, Arc<Tilt>>>> = OnceLock::new();
    let fits = FITS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut f = fits.lock().unwrap();
    f.entry(alpha.to_bits())
        .or_insert_with(|| {
            if alpha == 0.0 {
                return Arc::new(Tilt::from_theta([0.0; CELLS]));
            }
            let baked = super::tilts::BAKED.iter().find(|(a, _)| *a == alpha);
            Arc::new(match baked {
                Some((_, theta)) => Tilt::from_theta(*theta),
                None => Tilt::fit(alpha, 30, 200_000, 0xB5),
            })
        })
        .clone()
}

pub struct TiltedPlacer {
    pub alpha: f64,
    tilt: Arc<Tilt>,
}

impl TiltedPlacer {
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha,
            tilt: fitted(alpha),
        }
    }
}

impl Placer for TiltedPlacer {
    fn name(&self) -> String {
        format!("tilt{}", self.alpha)
    }
    fn place(&mut self, rng: &mut Rng) -> Layout {
        self.tilt.sample(rng)
    }
}

/// Picks a tilt at random each game: a mixed strategy in the placement game.
pub struct MixedPlacer {
    parts: Vec<(f64, f64, Arc<Tilt>)>, // (alpha, probability, tilt)
}

impl MixedPlacer {
    /// `parts`: (alpha, probability) pairs; probabilities are normalized.
    /// `None` unless they are finite, non-negative and sum to more than zero.
    pub fn new(parts: &[(f64, f64)]) -> Option<Self> {
        let total: f64 = parts.iter().map(|p| p.1).sum();
        let valid = parts.iter().all(|p| p.0.is_finite() && p.1 >= 0.0);
        if !valid || !total.is_finite() || total <= 0.0 {
            return None;
        }
        let parts = parts
            .iter()
            .map(|&(a, p)| (a, p / total, fitted(a)))
            .collect();
        Some(Self { parts })
    }

    /// Robust mix from the measured placement game (`broadside matrix`, 11
    /// tilts x 11 expectations, against both the exact and the density
    /// shooter families, 20-40k games per cell). Worst case over both families
    /// ~44.9 shots, versus 44.6 for uniform placement; shooters that assume
    /// uniform placement need ~45.4-45.7.
    pub fn equilibrium() -> Self {
        Self::new(&[(0.25, 0.45), (0.75, 0.30), (1.0, 0.25)]).expect("a valid mix")
    }
}

impl Placer for MixedPlacer {
    fn name(&self) -> String {
        let parts: Vec<String> = self
            .parts
            .iter()
            .map(|p| format!("{}={:.4}", p.0, p.1))
            .collect();
        format!("mix:{}", parts.join(","))
    }
    fn place(&mut self, rng: &mut Rng) -> Layout {
        let mut r = rng.f64();
        let i = self
            .parts
            .iter()
            .position(|p| {
                r -= p.1;
                r < 0.0
            })
            .unwrap_or(self.parts.len() - 1);
        self.parts[i].2.sample(rng)
    }
}

/// Equilibrium play that turns exploitative against opponents who keep
/// looking in the same places.
///
/// A game lasts until the opponent has found the last ship, so what a
/// layout is worth against a given way of searching is decided by its
/// best-hidden ship, not by its average cell. The placer records when the
/// opponent comes to each cell while hunting (shots aimed at a ship already
/// hit say nothing about where it looks unprompted), draws a few hundred
/// layouts and places one of those whose ships would be found latest.
///
/// A bandit chooses between that and the equilibrium mix by how many shots
/// the opponent spent per hit. Against an opponent that adapts, hiding stops
/// paying and the bandit drifts back to the equilibrium.
///
/// It learns only from shots actually fired: a game ends when either fleet
/// is sunk, so the opponent's trace usually stops short of sinking ours.
pub struct AdaptivePlacer {
    equilibrium: MixedPlacer,
    /// Per cell: the summed time (shots fired before it, as a share of the
    /// board) at which the opponent came to it while hunting.
    late: [f64; CELLS],
    games: u32,
    /// Plays and summed reward of the equilibrium mix and of hiding.
    arm_plays: [u32; 2],
    arm_shots: [f64; 2],
    last_arm: usize,
}

/// Layouts drawn per game when hiding.
const HIDE_DRAWS: usize = 600;
/// The layout placed is one of the best this many, so that the same hiding
/// place does not come up game after game.
const HIDE_TOP: u64 = 5;
/// What the average ship counts for next to the best-hidden one.
const HIDE_MEAN: f64 = 0.25;
/// Strength of the tilt toward late cells that a third of the draws come
/// from, in theta units per standard deviation of lateness.
const HIDE_TILT: f64 = 1.0;

impl Default for AdaptivePlacer {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptivePlacer {
    pub fn new() -> Self {
        Self {
            equilibrium: MixedPlacer::equilibrium(),
            late: [0.0; CELLS],
            games: 0,
            arm_plays: [0; 2],
            arm_shots: [0.0; 2],
            last_arm: 0,
        }
    }

    fn pick_arm(&self) -> usize {
        // Nothing seen yet: nowhere to hide.
        if self.games == 0 {
            return 0;
        }
        // UCB1 on the shots the opponent needs (as a fraction of the board).
        if let Some(a) = (0..2).find(|&a| self.arm_plays[a] == 0) {
            return a;
        }
        let total: f64 = self.arm_plays.iter().map(|&n| n as f64).sum();
        (0..2)
            .max_by(|&a, &b| {
                let ucb = |i: usize| {
                    let n = self.arm_plays[i] as f64;
                    self.arm_shots[i] / n + 0.15 * (2.0 * total.ln() / n).sqrt()
                };
                ucb(a).total_cmp(&ucb(b))
            })
            .unwrap()
    }

    /// When the opponent is expected to find each ship of `layout`, as a
    /// share of the board shot by then: the earliest of its cells.
    fn found(late: &[f64; CELLS], layout: &Layout) -> [f64; SHIPS] {
        crate::fleet::FLEET.map(|s| {
            Bits(layout.ship(s).0)
                .map(|c| late[c])
                .fold(f64::INFINITY, f64::min)
        })
    }

    /// A layout whose ships lie where the opponent has looked late.
    fn hide(&mut self, rng: &mut Rng) -> Layout {
        let late = self.late.map(|l| l / self.games as f64);
        let mu = late.iter().sum::<f64>() / CELLS as f64;
        let sd = (late.iter().map(|l| (l - mu).powi(2)).sum::<f64>() / CELLS as f64)
            .sqrt()
            .max(1e-6);
        let toward = Tilt::from_theta(late.map(|l| HIDE_TILT * (l - mu) / sd));
        // Draws from three sides: toward the late cells, from the
        // equilibrium mix, and from anywhere.
        let mut scored: Vec<(f64, Layout)> = (0..HIDE_DRAWS)
            .map(|i| {
                let layout = match i % 3 {
                    0 => toward.sample(rng),
                    1 => self.equilibrium.place(rng),
                    _ => Layout::random(rng),
                };
                let found = Self::found(&late, &layout);
                let last = found.iter().cloned().fold(0.0, f64::max);
                let mean = found.iter().sum::<f64>() / SHIPS as f64;
                (last + HIDE_MEAN * mean, layout)
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored[rng.below(HIDE_TOP.min(scored.len() as u64)) as usize].1
    }
}

impl Placer for AdaptivePlacer {
    fn name(&self) -> String {
        "adaptive".into()
    }

    fn place(&mut self, rng: &mut Rng) -> Layout {
        self.last_arm = self.pick_arm();
        match self.last_arm {
            0 => self.equilibrium.place(rng),
            _ => self.hide(rng),
        }
    }

    fn learn(&mut self, layout: &Layout, opponent_shots: &[usize]) {
        let ships = crate::fleet::FLEET.map(|s| layout.ship(s).0);
        let mut shot = 0u128;
        let mut time = [None; CELLS];
        for (i, &c) in opponent_shots.iter().enumerate() {
            let t = i as f64 / CELLS as f64;
            // With a ship hit and not yet sunk, the shot is aimed at it.
            // Like a cell never shot, it only says that hunting had not
            // come to the cell by then: halfway to the end is the guess.
            let aimed = ships.iter().any(|&m| m & shot != 0 && m & !shot != 0);
            time[c] = Some(if aimed { (t + 1.0) / 2.0 } else { t });
            shot |= 1 << c;
        }
        let end = opponent_shots.len() as f64 / CELLS as f64;
        for (l, t) in self.late.iter_mut().zip(time) {
            *l += t.unwrap_or((end + 1.0) / 2.0);
        }
        self.games += 1;
        // Shots per hit, scaled to the whole fleet: the shots the opponent
        // needed if it sank us, an estimate of them if the game ended first.
        let n = opponent_shots.len().max(1) as f64;
        let fleet = layout.occupied();
        let hits = opponent_shots.iter().filter(|&&c| fleet.has(c)).count();
        let needed = n * FLEET_CELLS as f64 / hits.max(1) as f64;
        self.arm_plays[self.last_arm] += 1;
        self.arm_shots[self.last_arm] += (needed / CELLS as f64).min(1.0);
    }
}

/// The adaptive placer before `AdaptivePlacer` learned to hide ship by
/// ship, kept to measure against: equilibrium play that tilts every cell
/// toward where the opponent looks late.
///
/// It records how late the opponent shoots each cell (never shooting a cell
/// counts as latest). The options are the equilibrium mix and two tilts toward
/// the cells the opponent reaches last; a UCB bandit picks among them by how
/// many shots the opponent spent per hit. Against an opponent that adapts,
/// exploitation stops paying and the bandit drifts back to the equilibrium.
///
/// It learns only from shots actually fired: a game ends when either fleet
/// is sunk, so the opponent's trace usually stops short of sinking ours.
pub struct LatenessPlacer {
    equilibrium: MixedPlacer,
    lateness: [f64; CELLS],
    games: u32,
    exploit: [Option<Tilt>; 2],
    arm_plays: [u32; 3],
    arm_shots: [f64; 3],
    last_arm: usize,
}

/// Strength of the exploiting tilts, in theta units per standard deviation of lateness.
const EXPLOIT_STRENGTH: [f64; 2] = [0.5, 1.0];
/// Games of pure equilibrium before exploitation is considered.
const WARMUP_GAMES: u32 = 4;

impl Default for LatenessPlacer {
    fn default() -> Self {
        Self::new()
    }
}

impl LatenessPlacer {
    pub fn new() -> Self {
        Self {
            equilibrium: MixedPlacer::equilibrium(),
            lateness: [0.0; CELLS],
            games: 0,
            exploit: [None, None],
            arm_plays: [0; 3],
            arm_shots: [0.0; 3],
            last_arm: 0,
        }
    }

    fn pick_arm(&self) -> usize {
        if self.games < WARMUP_GAMES {
            return 0;
        }
        // UCB1 on the shots the opponent needs (as a fraction of the board).
        if let Some(a) = (0..3).find(|&a| self.arm_plays[a] == 0) {
            return a;
        }
        let total: f64 = self.arm_plays.iter().map(|&n| n as f64).sum();
        (0..3)
            .max_by(|&a, &b| {
                let ucb = |i: usize| {
                    let n = self.arm_plays[i] as f64;
                    self.arm_shots[i] / n + 0.15 * (2.0 * total.ln() / n).sqrt()
                };
                ucb(a).total_cmp(&ucb(b))
            })
            .unwrap()
    }
}

impl Placer for LatenessPlacer {
    fn name(&self) -> String {
        "lateness".into()
    }

    fn place(&mut self, rng: &mut Rng) -> Layout {
        self.last_arm = self.pick_arm();
        match self.last_arm {
            0 => self.equilibrium.place(rng),
            a => self.exploit[a - 1]
                .as_ref()
                .expect("exploit tilts exist after warmup")
                .sample(rng),
        }
    }

    fn learn(&mut self, layout: &Layout, opponent_shots: &[usize]) {
        let n = opponent_shots.len().max(1) as f64;
        let mut late = [1.0; CELLS];
        for (i, &c) in opponent_shots.iter().enumerate() {
            late[c] = i as f64 / n;
        }
        for (l, x) in self.lateness.iter_mut().zip(late) {
            *l += x;
        }
        self.games += 1;
        // Shots per hit, scaled to the whole fleet: the shots the opponent
        // needed if it sank us, an estimate of them if the game ended first.
        let fleet = layout.occupied();
        let hits = opponent_shots.iter().filter(|&&c| fleet.has(c)).count();
        let needed = n * FLEET_CELLS as f64 / hits.max(1) as f64;
        self.arm_plays[self.last_arm] += 1;
        self.arm_shots[self.last_arm] += (needed / CELLS as f64).min(1.0);

        let mean_l: Vec<f64> = self
            .lateness
            .iter()
            .map(|l| l / self.games as f64)
            .collect();
        let mu = mean_l.iter().sum::<f64>() / CELLS as f64;
        let sd = (mean_l.iter().map(|x| (x - mu).powi(2)).sum::<f64>() / CELLS as f64)
            .sqrt()
            .max(1e-6);
        for (slot, strength) in self.exploit.iter_mut().zip(EXPLOIT_STRENGTH) {
            let mut theta = [0.0; CELLS];
            for (t, l) in theta.iter_mut().zip(&mean_l) {
                *t = strength * (l - mu) / sd;
            }
            *slot = Some(Tilt::from_theta(theta));
        }
    }
}

/// Cell occupancy of a placer, estimated from `n` samples.
pub fn occupancy(placer: &mut dyn Placer, n: usize, rng: &mut Rng) -> [f64; CELLS] {
    let mut occ = [0u64; CELLS];
    for _ in 0..n {
        for c in Bits(placer.place(rng).occupied().0) {
            occ[c] += 1;
        }
    }
    occ.map(|x| x as f64 / n as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_tilt_samples_legal_layouts() {
        let t = Tilt::from_theta([0.0; CELLS]);
        let mut rng = Rng::new(1);
        for _ in 0..200 {
            assert!(Layout::new(t.sample(&mut rng).slots).is_some());
        }
    }

    #[test]
    fn extreme_tilts_still_sample_legal_layouts() {
        let mut rng = Rng::new(2);
        // A constant shift is no tilt at all, however large.
        let mut one_cell = [0.0; CELLS];
        one_cell[44] = 60.0;
        for theta in [[-1000.0; CELLS], [1000.0; CELLS], one_cell] {
            let t = Tilt::from_theta(theta);
            for _ in 0..20 {
                assert!(Layout::new(t.sample(&mut rng).slots).is_some());
            }
        }
        let (flat, shifted) = (
            Tilt::from_theta([0.0; CELLS]),
            Tilt::from_theta([1.0; CELLS]),
        );
        for (a, b) in flat
            .weight
            .iter()
            .flatten()
            .zip(shifted.weight.iter().flatten())
        {
            assert!((a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn mixes_must_be_valid() {
        assert!(MixedPlacer::new(&[]).is_none());
        assert!(MixedPlacer::new(&[(1.0, 0.0)]).is_none());
        assert!(MixedPlacer::new(&[(1.0, -1.0), (0.0, 2.0)]).is_none());
        assert!(MixedPlacer::new(&[(1.0, f64::NAN)]).is_none());
        assert!(MixedPlacer::new(&[(0.0, 3.0), (1.0, 1.0)]).is_some());
    }

    #[test]
    fn flat_fit_flattens_occupancy() {
        let t = Tilt::fit(1.0, 12, 20_000, 3);
        let mut rng = Rng::new(9);
        let mut occ = [0u64; CELLS];
        let n = 40_000;
        for _ in 0..n {
            for c in Bits(t.sample(&mut rng).occupied().0) {
                occ[c] += 1;
            }
        }
        let corner = occ[0] as f64 / n as f64;
        let centre = occ[44] as f64 / n as f64;
        assert!(
            (corner - 0.17).abs() < 0.03 && (centre - 0.17).abs() < 0.03,
            "{corner} {centre}"
        );
    }
}

#[cfg(test)]
mod adaptive_tests {
    use super::*;

    #[test]
    fn adaptive_placers_stay_legal_while_learning() {
        let mut rng = Rng::new(4);
        let (mut new, mut old) = (AdaptivePlacer::new(), LatenessPlacer::new());
        for g in 0..30 {
            let (a, b) = (new.place(&mut rng), old.place(&mut rng));
            assert!(Layout::new(a.slots).is_some() && Layout::new(b.slots).is_some());
            // An opponent that always sweeps the board in reading order.
            let shots: Vec<usize> = (0..CELLS).take(60 + g).collect();
            new.learn(&a, &shots);
            old.learn(&b, &shots);
        }
        assert!(new.arm_plays.iter().sum::<u32>() == 30);
        assert!(old.arm_plays.iter().sum::<u32>() == 30);
    }

    #[test]
    fn hiding_puts_a_ship_where_the_opponent_looks_last() {
        let mut p = AdaptivePlacer::new();
        let mut rng = Rng::new(5);
        // The opponent sweeps the top half in reading order, then the game
        // is over: the bottom half is where it has not looked.
        let shots: Vec<usize> = (0..CELLS / 2).collect();
        for _ in 0..3 {
            let l = p.place(&mut rng);
            p.learn(&l, &shots);
        }
        for _ in 0..20 {
            let l = p.hide(&mut rng);
            let hidden = crate::fleet::FLEET
                .iter()
                .any(|&s| l.ship(s).iter().all(|c| c >= CELLS / 2));
            assert!(hidden, "{l:?}");
        }
    }

    #[test]
    fn aimed_shots_do_not_count_as_hunting() {
        let mut p = AdaptivePlacer::new();
        // Carrier along the top row; the opponent hits its first cell and
        // walks along it, then looks at the far corner.
        let layout = Layout::new([0, 40, 80, 120, 160]).unwrap();
        p.learn(&layout, &[0, 1, 2, 3, 4, 99]);
        // The first shot was a hunting shot at time 0, the last at 5/100.
        assert_eq!(p.late[0], 0.0);
        assert!((p.late[99] - 0.05).abs() < 1e-12);
        // The shots along the Carrier were aimed: later than their time.
        assert!(p.late[1] > 0.5 && p.late[4] > 0.5);
        // A cell never shot: halfway between the end of the game and never.
        assert!((p.late[50] - 0.53).abs() < 1e-12);
    }
}
