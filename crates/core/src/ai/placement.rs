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

    /// Fits θ by iterative proportional scaling on sampled occupancies.
    pub fn fit(alpha: f64, rounds: usize, samples: usize, seed: u64) -> Self {
        let p = Posterior::cached(&Knowledge::new(), 1);
        let u: Vec<f64> = (0..CELLS).map(|c| p.prob(c)).collect();
        let raw: Vec<f64> = u.iter().map(|x| x.powf(1.0 - alpha)).collect();
        let scale = FLEET_CELLS as f64 / raw.iter().sum::<f64>();
        let target: Vec<f64> = raw.iter().map(|x| x * scale).collect();

        let mut rng = Rng::new(seed);
        let mut tilt = Self::from_theta([0.0; CELLS]);
        for _ in 0..rounds {
            let mut occ = [0u64; CELLS];
            for _ in 0..samples {
                for c in Bits(tilt.sample(&mut rng).occupied().0) {
                    occ[c] += 1;
                }
            }
            let mut theta = tilt.theta;
            for c in 0..CELLS {
                let m = (occ[c] as f64 + 0.5) / samples as f64;
                theta[c] += 0.8 * (target[c] / m).ln();
            }
            tilt = Self::from_theta(theta);
        }
        tilt
    }
}

/// Shares fitted tilts between placers and shooters (fitting takes a moment).
pub fn fitted(alpha: f64) -> Arc<Tilt> {
    static FITS: OnceLock<Mutex<HashMap<u64, Arc<Tilt>>>> = OnceLock::new();
    let fits = FITS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut f = fits.lock().unwrap();
    f.entry(alpha.to_bits())
        .or_insert_with(|| Arc::new(Tilt::fit(alpha, 30, 200_000, 0xB5)))
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
