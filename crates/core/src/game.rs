//! Rules: fleet layouts, shot resolution, and what a shooter knows.

use crate::bitboard::{Bitboard, CELLS};
use crate::fleet::{FLEET, LENGTHS, SEGMENTS, SHIPS, SLOTS, Ship};
use crate::rng::Rng;

/// Result of a shot, as announced by the defender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Miss,
    Hit,
    Sunk(Ship),
}

/// A complete fleet: one placement slot per ship (see `fleet` module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layout {
    pub slots: [u8; SHIPS],
}

impl Layout {
    /// Validates slots: every ship on the board, no overlaps.
    pub fn new(slots: [u8; SHIPS]) -> Option<Self> {
        let mut seen = 0u128;
        for (s, &slot) in slots.iter().enumerate() {
            let m = *SEGMENTS[LENGTHS[s]].get(slot as usize)?;
            if m == 0 || m & seen != 0 {
                return None;
            }
            seen |= m;
        }
        Some(Self { slots })
    }

    /// Uniformly random legal layout: independent placements, rejected on overlap.
    pub fn random(rng: &mut Rng) -> Self {
        loop {
            let mut slots = [0u8; SHIPS];
            let mut seen = 0u128;
            let mut ok = true;
            for (s, slot) in slots.iter_mut().enumerate() {
                let m = loop {
                    let k = rng.below(SLOTS as u64) as usize;
                    if SEGMENTS[LENGTHS[s]][k] != 0 {
                        *slot = k as u8;
                        break SEGMENTS[LENGTHS[s]][k];
                    }
                };
                ok &= m & seen == 0;
                seen |= m;
            }
            if ok {
                return Self { slots };
            }
        }
    }

    #[inline]
    pub fn ship(&self, s: Ship) -> Bitboard {
        Bitboard(SEGMENTS[s.size()][self.slots[s.index()] as usize])
    }

    pub fn occupied(&self) -> Bitboard {
        FLEET
            .iter()
            .fold(Bitboard::EMPTY, |acc, &s| acc | self.ship(s))
    }

    /// How close the ships come: 0 if they all lie apart, 1 if some meet
    /// at a corner but none share an edge, 2 if some share an edge.
    pub fn contact(&self) -> usize {
        let mut kind = 0;
        for (i, &a) in FLEET.iter().enumerate() {
            for &b in &FLEET[..i] {
                let (a, b) = (self.ship(a).0, self.ship(b).0);
                if crate::bitboard::neighbours4(a) & b != 0 {
                    return 2;
                }
                if crate::bitboard::grow8(a) & b != 0 {
                    kind = 1;
                }
            }
        }
        kind
    }

    pub fn ship_at(&self, cell: usize) -> Option<Ship> {
        FLEET.into_iter().find(|&s| self.ship(s).has(cell))
    }
}

/// The defending side: a layout plus the shots received so far.
#[derive(Clone, Debug)]
pub struct Board {
    pub layout: Layout,
    pub shots: Bitboard,
}

impl Board {
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            shots: Bitboard::EMPTY,
        }
    }

    /// Resolves a shot. Repeat shots are reported as misses (they add nothing).
    pub fn fire(&mut self, cell: usize) -> Outcome {
        debug_assert!(cell < CELLS);
        let fresh = !self.shots.has(cell);
        self.shots |= Bitboard::bit(cell);
        match self.layout.ship_at(cell) {
            Some(s) if fresh => {
                let m = self.layout.ship(s);
                if m.0 & !self.shots.0 == 0 {
                    Outcome::Sunk(s)
                } else {
                    Outcome::Hit
                }
            }
            _ => Outcome::Miss,
        }
    }

    pub fn defeated(&self) -> bool {
        self.layout.occupied().0 & !self.shots.0 == 0
    }
}

/// How a ship was sunk: the finishing shot, and every hit standing at that
/// moment (the ship lies inside those hits and contains `cell`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sinking {
    pub cell: u8,
    pub hits: Bitboard,
}

/// Everything the shooter has learned. This alone determines the posterior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Knowledge {
    pub misses: Bitboard,
    pub hits: Bitboard,
    pub sunk: [Option<Sinking>; SHIPS],
}

impl Knowledge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, cell: usize, outcome: Outcome) {
        let b = Bitboard::bit(cell);
        match outcome {
            Outcome::Miss => self.misses |= b,
            Outcome::Hit => self.hits |= b,
            Outcome::Sunk(s) => {
                self.hits |= b;
                self.sunk[s.index()] = Some(Sinking {
                    cell: cell as u8,
                    hits: self.hits,
                });
            }
        }
    }

    #[inline]
    pub fn shots(&self) -> Bitboard {
        self.misses | self.hits
    }

    #[inline]
    pub fn unknown(&self) -> Bitboard {
        !self.shots()
    }

    pub fn shots_fired(&self) -> u32 {
        self.shots().count()
    }

    pub fn is_sunk(&self, s: Ship) -> bool {
        self.sunk[s.index()].is_some()
    }

    /// Bit `i` set when ship `i` is still afloat.
    pub fn afloat_mask(&self) -> u8 {
        (0..SHIPS)
            .filter(|&i| self.sunk[i].is_none())
            .fold(0, |m, i| m | 1 << i)
    }

    pub fn won(&self) -> bool {
        self.sunk.iter().all(Option::is_some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_layouts_are_legal() {
        let mut rng = Rng::new(3);
        for _ in 0..1000 {
            let l = Layout::random(&mut rng);
            assert!(Layout::new(l.slots).is_some());
            assert_eq!(l.occupied().count(), 17);
        }
    }

    #[test]
    fn rejects_overlap_and_off_board() {
        // Carrier horizontal at A1, Battleship vertical at A1: overlap.
        assert!(Layout::new([0, 1, 40, 60, 80]).is_none());
        // Carrier horizontal starting at column 7 runs off the board.
        assert!(Layout::new([2 * 7, 1 + 2 * 20, 40, 60, 80]).is_none());
    }

    #[test]
    fn firing_reports_hits_and_sinks() {
        let mut rng = Rng::new(11);
        let layout = Layout::random(&mut rng);
        let mut board = Board::new(layout);
        let mut k = Knowledge::new();
        let destroyer = layout.ship(Ship::Destroyer);
        let cells: Vec<_> = destroyer.iter().collect();
        assert_eq!(board.fire(cells[0]), Outcome::Hit);
        k.record(cells[0], Outcome::Hit);
        assert_eq!(board.fire(cells[1]), Outcome::Sunk(Ship::Destroyer));
        k.record(cells[1], Outcome::Sunk(Ship::Destroyer));
        assert!(k.is_sunk(Ship::Destroyer));
        assert_eq!(k.sunk[4].unwrap().hits, destroyer);
        for cell in 0..CELLS {
            board.fire(cell);
        }
        assert!(board.defeated());
    }
}
