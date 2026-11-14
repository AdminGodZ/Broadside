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

/// Per-ship cumulative placement weights for fast sampling.
#[derive(Clone)]
pub struct Tilt {
    pub theta: [f64; CELLS],
    /// Relative weight of each placement, by ship and slot (0 off the board).
    pub weight: [[f64; SLOTS]; SHIPS],
    cdf: [[f64; SLOTS]; SHIPS],
}

impl Tilt {
    pub fn from_theta(theta: [f64; CELLS]) -> Self {
        let mut weight = [[0.0; SLOTS]; SHIPS];
        let mut cdf = [[0.0; SLOTS]; SHIPS];
        for s in 0..SHIPS {
            let mut acc = 0.0;
            for slot in 0..SLOTS {
                let m = SEGMENTS[LENGTHS[s]][slot];
                if m != 0 {
                    weight[s][slot] = Bits(m).map(|x| theta[x]).sum::<f64>().exp();
                }
                acc += weight[s][slot];
                cdf[s][slot] = acc;
            }
        }
        Self { theta, weight, cdf }
    }

    pub fn sample(&self, rng: &mut Rng) -> Layout {
        loop {
            let mut slots = [0u8; SHIPS];
            let mut used = 0u128;
            let mut ok = true;
            for (s, slot) in slots.iter_mut().enumerate() {
                let cdf = &self.cdf[s];
                let r = rng.f64() * cdf[SLOTS - 1];
                let k = cdf.partition_point(|&c| c <= r).min(SLOTS - 1);
                *slot = k as u8;
                let m = SEGMENTS[LENGTHS[s]][k];
                ok &= m & used == 0;
                used |= m;
            }
            if ok {
                return Layout { slots };
            }
        }
    }

    /// Target occupancy of tilt `alpha`: `u^(1-alpha)`, scaled to 17 ship cells.
    pub fn target_occupancy(alpha: f64) -> [f64; CELLS] {
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
    parts: Vec<(f64, f64, Arc<Tilt>)>, // (alpha, cumulative probability, tilt)
}

impl MixedPlacer {
    /// `parts`: (alpha, probability) pairs; probabilities are normalized.
    pub fn new(parts: &[(f64, f64)]) -> Self {
        let total: f64 = parts.iter().map(|p| p.1).sum();
        let mut acc = 0.0;
        let parts = parts
            .iter()
            .map(|&(a, p)| {
                acc += p / total;
                (a, acc, fitted(a))
            })
            .collect();
        Self { parts }
    }

    /// Robust mix from the measured placement game (`broadside matrix`, 11
    /// tilts x 11 expectations, against both the exact and the density
    /// shooter families, 20-40k games per cell). Worst case over both families
    /// ~44.9 shots, versus 44.6 for uniform placement; shooters that assume
    /// uniform placement need ~45.4-45.7.
    pub fn equilibrium() -> Self {
        Self::new(&[(0.25, 0.45), (0.75, 0.30), (1.0, 0.25)])
    }
}

impl Placer for MixedPlacer {
    fn name(&self) -> String {
        let parts: Vec<String> = self.parts.iter().map(|p| p.0.to_string()).collect();
        format!("mix({})", parts.join(","))
    }
    fn place(&mut self, rng: &mut Rng) -> Layout {
        let r = rng.f64();
        let i = self
            .parts
            .iter()
            .position(|p| r < p.1)
            .unwrap_or(self.parts.len() - 1);
        self.parts[i].2.sample(rng)
    }
}

/// Equilibrium play that turns exploitative against opponents who keep
/// looking in the same places.
///
/// It records how late the opponent shoots each cell (never shooting a cell
/// counts as latest). The options are the equilibrium mix and two tilts toward
/// the cells the opponent reaches last; a UCB bandit picks among them by the
/// shots the opponent needed. Against an opponent that adapts, exploitation
/// stops paying and the bandit drifts back to the equilibrium.
pub struct AdaptivePlacer {
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

impl Default for AdaptivePlacer {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptivePlacer {
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
        // UCB1 on shots the opponent needed (scaled to roughly [0, 1]).
        if let Some(a) = (0..3).find(|&a| self.arm_plays[a] == 0) {
            return a;
        }
        let total: f64 = self.arm_plays.iter().map(|&n| n as f64).sum();
        (0..3)
            .max_by(|&a, &b| {
                let ucb = |i: usize| {
                    let n = self.arm_plays[i] as f64;
                    self.arm_shots[i] / n / 100.0 + 0.15 * (2.0 * total.ln() / n).sqrt()
                };
                ucb(a).total_cmp(&ucb(b))
            })
            .unwrap()
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
            a => self.exploit[a - 1]
                .as_ref()
                .expect("exploit tilts exist after warmup")
                .sample(rng),
        }
    }

    fn learn(&mut self, _layout: &Layout, opponent_shots: &[usize]) {
        let n = opponent_shots.len().max(1) as f64;
        let mut late = [1.0; CELLS];
        for (i, &c) in opponent_shots.iter().enumerate() {
            late[c] = i as f64 / n;
        }
        for (l, x) in self.lateness.iter_mut().zip(late) {
            *l += x;
        }
        self.games += 1;
        self.arm_plays[self.last_arm] += 1;
        self.arm_shots[self.last_arm] += opponent_shots.len() as f64;

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
    fn adaptive_placer_stays_legal_while_learning() {
        let mut p = AdaptivePlacer::new();
        let mut rng = Rng::new(4);
        for g in 0..30 {
            let l = p.place(&mut rng);
            assert!(Layout::new(l.slots).is_some());
            // An opponent that always sweeps the board in reading order.
            let shots: Vec<usize> = (0..CELLS).take(60 + g).collect();
            p.learn(&l, &shots);
        }
        assert!(p.arm_plays.iter().sum::<u32>() == 30);
    }
}
