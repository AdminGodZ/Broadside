//! The board's eight symmetries (rotations and reflections), used to share
//! work between positions that are mirror images of each other.

use crate::bitboard::{Bitboard, Bits, CELLS, SIZE};
use crate::fleet::{SEGMENTS, SHIPS, SLOTS};
use crate::game::{Knowledge, Sinking};

pub const TRANSFORMS: usize = 8;

const fn map_cell(t: usize, cell: usize) -> usize {
    let (r, c) = (cell / SIZE, cell % SIZE);
    let (r, c) = if t & 4 != 0 { (c, r) } else { (r, c) };
    let r = if t & 1 != 0 { SIZE - 1 - r } else { r };
    let c = if t & 2 != 0 { SIZE - 1 - c } else { c };
    r * SIZE + c
}

const fn build_cells() -> [[u8; CELLS]; TRANSFORMS] {
    let mut m = [[0u8; CELLS]; TRANSFORMS];
    let mut t = 0;
    while t < TRANSFORMS {
        let mut c = 0;
        while c < CELLS {
            m[t][c] = map_cell(t, c) as u8;
            c += 1;
        }
        t += 1;
    }
    m
}

/// `CELL_MAP[t][cell]`: image of `cell` under transform `t`.
pub static CELL_MAP: [[u8; CELLS]; TRANSFORMS] = build_cells();

const fn map_mask_const(t: usize, m: u128) -> u128 {
    let mut out = 0u128;
    let mut c = 0;
    while c < CELLS {
        if m >> c & 1 == 1 {
            out |= 1 << map_cell(t, c);
        }
        c += 1;
    }
    out
}

const fn build_slots() -> [[[u8; SLOTS]; 6]; TRANSFORMS] {
    let mut m = [[[0u8; SLOTS]; 6]; TRANSFORMS];
    let mut t = 0;
    while t < TRANSFORMS {
        let mut len = 2;
        while len <= 5 {
            let mut slot = 0;
            while slot < SLOTS {
                let seg = SEGMENTS[len][slot];
                if seg != 0 {
                    let img = map_mask_const(t, seg);
                    let start = img.trailing_zeros() as usize;
                    let vertical = img >> (start + SIZE) & 1 == 1;
                    m[t][len][slot] = (2 * start + vertical as usize) as u8;
                }
                slot += 1;
            }
            len += 1;
        }
        t += 1;
    }
    m
}

/// `SLOT_MAP[t][len][slot]`: image of a placement under transform `t`.
pub static SLOT_MAP: [[[u8; SLOTS]; 6]; TRANSFORMS] = build_slots();

#[inline]
pub fn map_mask(t: usize, m: Bitboard) -> Bitboard {
    if t == 0 {
        return m;
    }
    Bitboard(Bits(m.0).fold(0, |acc, c| acc | 1 << CELL_MAP[t][c]))
}

pub fn map_knowledge(t: usize, k: &Knowledge) -> Knowledge {
    let mut out = Knowledge {
        misses: map_mask(t, k.misses),
        hits: map_mask(t, k.hits),
        sunk: [None; SHIPS],
    };
    for (o, s) in out.sunk.iter_mut().zip(k.sunk.iter()) {
        *o = s.map(|s| Sinking {
            cell: CELL_MAP[t][s.cell as usize],
            hits: map_mask(t, s.hits),
        });
    }
    out
}

/// Ordering key: canonical form is the transform with the smallest key.
fn key(k: &Knowledge) -> (u128, u128, [(u8, u128); SHIPS]) {
    let sunk = k.sunk.map(|s| s.map_or((255, 0), |s| (s.cell, s.hits.0)));
    (k.misses.0, k.hits.0, sunk)
}

/// The canonical image of `k` and the transform that produces it.
pub fn canonical(k: &Knowledge) -> (Knowledge, usize) {
    let mut best = (*k, 0);
    let mut best_key = key(k);
    for t in 1..TRANSFORMS {
        let img = map_knowledge(t, k);
        let kk = key(&img);
        if kk < best_key {
            best = (img, t);
            best_key = kk;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_are_permutations() {
        for t in 0..TRANSFORMS {
            let mut seen = [false; CELLS];
            for c in 0..CELLS {
                seen[CELL_MAP[t][c] as usize] = true;
            }
            assert!(seen.iter().all(|&s| s));
            for len in 2..=5 {
                for slot in 0..SLOTS {
                    let seg = SEGMENTS[len][slot];
                    if seg != 0 {
                        let img = SEGMENTS[len][SLOT_MAP[t][len][slot] as usize];
                        assert_eq!(map_mask(t, Bitboard(seg)).0, img);
                    }
                }
            }
        }
    }

    #[test]
    fn canonical_is_shared_by_images() {
        let mut k = Knowledge::new();
        k.misses = Bitboard(1 << 3 | 1 << 47);
        k.hits = Bitboard(1 << 55);
        let (c, _) = canonical(&k);
        for t in 0..TRANSFORMS {
            assert_eq!(canonical(&map_knowledge(t, &k)).0, c);
        }
    }
}
