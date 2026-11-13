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

use super::{Shooter, argmax_random};
use crate::bitboard::Bits;
use crate::fleet::{LENGTHS, SEGMENTS, SHIPS, Ship, THROUGH};
use crate::game::{Knowledge, Outcome};
use crate::posterior::Posterior;
use crate::rng::Rng;

#[derive(Clone, Copy, Debug)]
pub struct Search {
    /// Rollouts run while at most this many ships are afloat (0 = never).
    pub max_afloat: usize,
    /// Most likely cells considered.
    pub candidates: usize,
    /// Sampled layouts per decision when the posterior is not enumerated.
    pub samples: usize,
}

pub struct Broadside {
    pub threads: usize,
    pub search: Search,
    /// Hunting score weight per ship (1.0 each = plain hit probability).
    pub hunt_weights: [f64; SHIPS],
}

impl Broadside {
    pub fn new() -> Self {
        Self {
            threads: 1,
            search: Search {
                max_afloat: 1,
                candidates: 8,
                samples: 64,
            },
            hunt_weights: [1.0; SHIPS],
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

/// Plain greedy choice: the most likely cell, ties broken at random.
pub fn greedy_choice(k: &Knowledge, p: &Posterior, rng: &mut Rng) -> usize {
    argmax_random(k.unknown().0, rng, |c| p.cells[c])
}

/// Greedy with a fixed tie-break (lowest cell). Rollouts need a deterministic
/// base policy: with random ties each evaluation is a noisy draw, and picking
/// the best of several noisy estimates selects noise, not better shots.
pub fn greedy_det(k: &Knowledge, p: &Posterior) -> usize {
    let mut best = (0, usize::MAX);
    for c in Bits(k.unknown().0) {
        if p.cells[c] > best.0 || best.1 == usize::MAX {
            best = (p.cells[c], c);
        }
    }
    best.1
}

/// Shots to finish the game from `k`, firing `first` and then greedily,
/// when the afloat ships truly lie at `truth` (slot per ship).
fn rollout(k: &Knowledge, first: usize, truth: &[Option<u8>; SHIPS]) -> u32 {
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
        cell = greedy_det(&k, &Posterior::compute(&k));
    }
}

impl Broadside {
    fn search(&self, k: &Knowledge, p: &Posterior, rng: &mut Rng) -> usize {
        let unknown = k.unknown().0;
        let mut cands: Vec<usize> = Bits(unknown).collect();
        cands.sort_by_key(|&c| std::cmp::Reverse(p.cells[c]));
        cands.truncate(self.search.candidates);

        // The futures to evaluate against: (afloat placements, weight).
        let afloat: Vec<usize> = Bits(k.afloat_mask() as u128).collect();
        let futures: Vec<([Option<u8>; SHIPS], u64)> = if let [ship] = afloat[..] {
            (0..p.ships[ship].len())
                .filter(|&slot| p.ships[ship][slot] > 0)
                .map(|slot| {
                    let mut t = [None; SHIPS];
                    t[ship] = Some(slot as u8);
                    (t, p.ships[ship][slot])
                })
                .collect()
        } else {
            (0..self.search.samples)
                .map(|_| (Posterior::sample_afloat(k, rng), 1))
                .collect()
        };

        let mut best = (f64::INFINITY, cands[0]);
        for &x in &cands {
            let mut total = 0.0;
            for (truth, w) in &futures {
                total += rollout(k, x, truth) as f64 * *w as f64;
            }
            if total < best.0 - 1e-9 {
                best = (total, x);
            }
        }
        best.1
    }
}

impl Shooter for Broadside {
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
        if (k.afloat_mask().count_ones() as usize) <= self.search.max_afloat {
            return self.search(k, &p, rng);
        }
        if self.hunt_weights == [1.0; SHIPS] {
            return greedy_choice(k, &p, rng);
        }
        let sc = p.ship_cells();
        let w = self.hunt_weights;
        let mut best = (f64::NEG_INFINITY, 0u128);
        for c in Bits(unknown) {
            let v: f64 = (0..SHIPS).map(|s| w[s] * sc[s][c] as f64).sum();
            if v > best.0 * (1.0 + 1e-12) {
                best = (v, 1 << c);
            } else if v >= best.0 * (1.0 - 1e-12) {
                best.1 |= 1 << c;
            }
        }
        rng.pick_bit(best.1)
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
