//! Classic opponents to measure against. They reason the way typical bots do:
//! heuristically, without exact inference.

use super::{Shooter, argmax_random};
use crate::bitboard::{Bits, CELLS, SIZE, neighbours4};
use crate::fleet::{LENGTHS, SEGMENTS, SHIPS, THROUGH};
use crate::game::Knowledge;
use crate::rng::Rng;

/// Fires at a uniformly random unshot cell.
pub struct RandomShooter;

impl Shooter for RandomShooter {
    fn name(&self) -> String {
        "random".into()
    }
    fn choose(&mut self, k: &Knowledge, rng: &mut Rng) -> usize {
        rng.pick_bit(k.unknown().0)
    }
}

/// Best guess at which hits belong to sunk ships: each sunk ship (in sinking
/// order) takes its first placement through the finishing shot that lies
/// inside the hits standing then. Heuristic bots reason this way; it can be
/// wrong when sunk ships touch.
pub fn guess_sunk_cells(k: &Knowledge) -> u128 {
    let mut order: Vec<usize> = (0..SHIPS).filter(|&s| k.sunk[s].is_some()).collect();
    order.sort_by_key(|&s| k.sunk[s].unwrap().hits.count());
    let mut taken = 0u128;
    for s in order {
        let sk = k.sunk[s].unwrap();
        let len = LENGTHS[s];
        let t = THROUGH[len][sk.cell as usize];
        let fit = t.slots[..t.n as usize]
            .iter()
            .map(|&slot| SEGMENTS[len][slot as usize])
            .find(|&m| m & !(sk.hits.0 & !taken) == 0);
        taken |= fit.unwrap_or(1 << sk.cell);
    }
    taken
}

/// Hunt randomly (optionally on a checkerboard); after a hit, probe its
/// neighbours, extending lines of two or more hits first.
pub struct HuntTarget {
    pub parity: bool,
}

impl Shooter for HuntTarget {
    fn name(&self) -> String {
        if self.parity {
            "parity".into()
        } else {
            "hunt-target".into()
        }
    }

    fn choose(&mut self, k: &Knowledge, rng: &mut Rng) -> usize {
        let unknown = k.unknown().0;
        let active = k.hits.0 & !guess_sunk_cells(k);
        if active != 0 {
            // Cells that extend two adjacent active hits along their line.
            let h_pairs = active & (active >> 1) & crate::bitboard::NOT_COL9;
            let v_pairs = active & (active >> SIZE);
            let h_line = h_pairs | h_pairs << 1;
            let v_line = v_pairs | v_pairs << SIZE;
            let ext = ((h_line << 1 & crate::bitboard::NOT_COL0)
                | (h_line >> 1 & crate::bitboard::NOT_COL9))
                | (v_line << SIZE | v_line >> SIZE);
            let ext = ext & unknown;
            if ext != 0 {
                return rng.pick_bit(ext);
            }
            let near = neighbours4(active) & unknown;
            if near != 0 {
                return rng.pick_bit(near);
            }
        }
        if self.parity {
            let smallest = (0..SHIPS)
                .filter(|&s| k.sunk[s].is_none())
                .map(|s| LENGTHS[s])
                .min()
                .unwrap_or(2);
            let lattice: u128 = (0..CELLS)
                .filter(|&c| (c / SIZE + c % SIZE).is_multiple_of(smallest))
                .fold(0, |m, c| m | 1 << c);
            if unknown & lattice != 0 {
                return rng.pick_bit(unknown & lattice);
            }
        }
        rng.pick_bit(unknown)
    }
}

/// The well-known probability-density bot: for every ship still afloat, count
/// placements that avoid misses and (guessed) sunk ships; placements through
/// unresolved hits count far more. Fire at the densest cell.
pub struct Density;

impl Shooter for Density {
    fn name(&self) -> String {
        "density".into()
    }

    fn choose(&mut self, k: &Knowledge, rng: &mut Rng) -> usize {
        let unknown = k.unknown().0;
        let sunk_cells = guess_sunk_cells(k);
        let active = k.hits.0 & !sunk_cells;
        let blocked = k.misses.0 | sunk_cells;
        let mut score = [0u64; CELLS];
        for s in (0..SHIPS).filter(|&s| k.sunk[s].is_none()) {
            let len = LENGTHS[s];
            for &m in &SEGMENTS[len] {
                if m == 0 || m & blocked != 0 {
                    continue;
                }
                let through = (m & active).count_ones();
                let w = if active == 0 {
                    1
                } else if through == 0 {
                    0
                } else {
                    100u64.pow(through)
                };
                for c in Bits(m) {
                    score[c] += w;
                }
            }
        }
        argmax_random(unknown, rng, |c| score[c])
    }
}
