//! The standard fleet and every legal position each ship can occupy.
//!
//! Placements are indexed `start * 2 + vertical`, where `start` is the
//! top-left cell. That gives 200 slots per ship length; slots whose ship would
//! run off the board hold an empty mask and never appear in start masks.

use crate::bitboard::{Bitboard, CELLS, SIZE};
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ship {
    Carrier,
    Battleship,
    Cruiser,
    Submarine,
    Destroyer,
}

pub const SHIPS: usize = 5;

/// Ship lengths by `Ship::index`.
pub const LENGTHS: [usize; SHIPS] = [5, 4, 3, 3, 2];

/// Cells occupied by a full fleet.
pub const FLEET_CELLS: u32 = 17;

impl Ship {
    pub const fn size(self) -> usize {
        LENGTHS[self.index()]
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn from_index(i: usize) -> Ship {
        FLEET[i]
    }

    pub const fn name(self) -> &'static str {
        match self {
            Ship::Carrier => "Carrier",
            Ship::Battleship => "Battleship",
            Ship::Cruiser => "Cruiser",
            Ship::Submarine => "Submarine",
            Ship::Destroyer => "Destroyer",
        }
    }
}

/// Largest first: placing big ships early prunes overlap searches fastest.
pub const FLEET: [Ship; SHIPS] = [
    Ship::Carrier,
    Ship::Battleship,
    Ship::Cruiser,
    Ship::Submarine,
    Ship::Destroyer,
];

/// Number of placement slots per ship length (see module docs).
pub const SLOTS: usize = 2 * CELLS;

/// Mask of a `len`-cell ship starting at `start`, or 0 if it leaves the board.
pub const fn segment(len: usize, start: usize, vertical: bool) -> u128 {
    let (row, col) = (start / SIZE, start % SIZE);
    if (vertical && row + len > SIZE) || (!vertical && col + len > SIZE) {
        return 0;
    }
    let step = if vertical { SIZE } else { 1 };
    let mut m = 0u128;
    let mut k = 0;
    while k < len {
        m |= 1 << (start + k * step);
        k += 1;
    }
    m
}

const fn build_segments() -> [[u128; SLOTS]; 6] {
    let mut t = [[0u128; SLOTS]; 6];
    let mut len = 2;
    while len <= 5 {
        let mut slot = 0;
        while slot < SLOTS {
            t[len][slot] = segment(len, slot / 2, slot % 2 == 1);
            slot += 1;
        }
        len += 1;
    }
    t
}

/// `SEGMENTS[len][slot]`: mask of every placement, by length and slot.
pub static SEGMENTS: [[u128; SLOTS]; 6] = build_segments();

const fn build_starts(vertical: bool) -> [u128; 6] {
    let mut t = [0u128; 6];
    let mut len = 2;
    while len <= 5 {
        let mut cell = 0;
        while cell < CELLS {
            if segment(len, cell, vertical) != 0 {
                t[len] |= 1 << cell;
            }
            cell += 1;
        }
        len += 1;
    }
    t
}

/// Cells where a horizontal ship of each length may start.
pub const H_STARTS: [u128; 6] = build_starts(false);
/// Cells where a vertical ship of each length may start.
pub const V_STARTS: [u128; 6] = build_starts(true);

/// Slots of every `len`-ship placement that covers a given cell.
#[derive(Clone, Copy)]
pub struct Through {
    pub n: u8,
    pub slots: [u8; 10],
}

const fn build_through() -> [[Through; CELLS]; 6] {
    let mut t = [[Through {
        n: 0,
        slots: [0; 10],
    }; CELLS]; 6];
    let mut len = 2;
    while len <= 5 {
        let mut slot = 0;
        while slot < SLOTS {
            let m = segment(len, slot / 2, slot % 2 == 1);
            let mut cell = 0;
            while cell < CELLS {
                if m >> cell & 1 == 1 {
                    let e = &mut t[len][cell];
                    e.slots[e.n as usize] = slot as u8;
                    e.n += 1;
                }
                cell += 1;
            }
            slot += 1;
        }
        len += 1;
    }
    t
}

/// `THROUGH[len][cell]`: placements of a `len`-ship covering `cell`.
pub static THROUGH: [[Through; CELLS]; 6] = build_through();

/// Horizontal and vertical start masks of every `len`-ship that fits in `free`.
#[inline(always)]
pub fn starts(len: usize, free: u128) -> (u128, u128) {
    let (mut h, mut v) = (free, free);
    for k in 1..len {
        h &= free >> k;
        v &= free >> (SIZE * k);
    }
    (h & H_STARTS[len], v)
}

/// Every horizontal and vertical position of a ship `len` cells long.
pub fn placements(len: usize) -> Vec<Bitboard> {
    (0..SLOTS)
        .map(|slot| SEGMENTS[len][slot])
        .filter(|&m| m != 0)
        .map(Bitboard)
        .collect()
}

/// Exact number of fleet layouts (ships may touch, not overlap), treating the
/// Cruiser and Submarine as distinct ships. Brute force across all cores.
pub fn count_layouts() -> u64 {
    let [p5, p4, p3, p2] = [5, 4, 3, 2].map(placements);
    let threads = thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = p5.len().div_ceil(threads);

    thread::scope(|s| {
        let workers: Vec<_> = p5
            .chunks(chunk)
            .map(|carriers| {
                let (p4, p3, p2) = (&p4, &p3, &p2);
                s.spawn(move || {
                    let mut n = 0u64;
                    for &a in carriers {
                        for &b in p4.iter().filter(|b| !a.intersects(**b)) {
                            let ab = a | b;
                            for &c in p3.iter().filter(|c| !ab.intersects(**c)) {
                                let abc = ab | c;
                                for &d in p3.iter().filter(|d| !abc.intersects(**d)) {
                                    let abcd = abc | d;
                                    n += p2.iter().filter(|e| !abcd.intersects(**e)).count() as u64;
                                }
                            }
                        }
                    }
                    n
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).sum()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_counts() {
        let counts = FLEET.map(|s| placements(s.size()).len());
        assert_eq!(counts, [120, 140, 160, 160, 180]);
    }

    #[test]
    fn placements_are_straight_and_sized() {
        for len in 2..=5 {
            for p in placements(len) {
                assert_eq!(p.count() as usize, len);
                let cells: Vec<_> = (0..100).filter(|i| p.0 >> i & 1 == 1).collect();
                let same_row = cells.iter().all(|c| c / SIZE == cells[0] / SIZE);
                let same_col = cells.iter().all(|c| c % SIZE == cells[0] % SIZE);
                assert!(same_row || same_col, "ship wraps across rows:\n{p}");
            }
        }
    }

    #[test]
    fn start_masks_match_tables() {
        for len in 2..=5 {
            let (h, v) = starts(len, Bitboard::FULL.0);
            assert_eq!(h, H_STARTS[len]);
            assert_eq!(v, V_STARTS[len]);
            assert_eq!(
                (h.count_ones() + v.count_ones()) as usize,
                placements(len).len()
            );
        }
    }

    #[test]
    fn through_lists_cover_cell() {
        for len in 2..=5 {
            for (cell, &t) in THROUGH[len].iter().enumerate() {
                for &slot in &t.slots[..t.n as usize] {
                    assert!(SEGMENTS[len][slot as usize] >> cell & 1 == 1);
                }
                let expect = (0..SLOTS)
                    .filter(|&s| SEGMENTS[len][s] >> cell & 1 == 1)
                    .count();
                assert_eq!(t.n as usize, expect);
            }
        }
    }

    /// Slow in debug builds: `cargo test --release -- --ignored`
    #[test]
    #[ignore]
    fn total_layouts() {
        assert_eq!(count_layouts(), 30_093_975_536);
    }
}
