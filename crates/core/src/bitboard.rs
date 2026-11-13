//! A 10x10 board packed into the low 100 bits of a `u128`.
//! Bit index = `row * 10 + col`. Overlap tests and counts are single instructions.

use std::fmt;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not};

pub const SIZE: usize = 10;
pub const CELLS: usize = SIZE * SIZE;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bitboard(pub u128);

impl Bitboard {
    pub const EMPTY: Self = Self(0);
    pub const FULL: Self = Self((1u128 << CELLS) - 1);

    #[inline]
    pub const fn cell(row: usize, col: usize) -> Self {
        Self(1 << (row * SIZE + col))
    }

    #[inline]
    pub const fn contains(self, row: usize, col: usize) -> bool {
        self.0 & Self::cell(row, col).0 != 0
    }

    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    #[inline]
    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn bit(index: usize) -> Self {
        Self(1 << index)
    }

    #[inline]
    pub const fn has(self, index: usize) -> bool {
        self.0 >> index & 1 == 1
    }

    /// Indices of the set cells, lowest first.
    #[inline]
    pub fn iter(self) -> Bits {
        Bits(self.0)
    }
}

const fn column_mask(skip: usize) -> u128 {
    let mut x = 0u128;
    let mut i = 0;
    while i < CELLS {
        if i % SIZE != skip {
            x |= 1 << i;
        }
        i += 1;
    }
    x
}

/// Every cell except column 1 / column 10.
pub const NOT_COL0: u128 = column_mask(0);
pub const NOT_COL9: u128 = column_mask(SIZE - 1);

/// Orthogonal neighbours of the cells in `m` (excluding `m` itself unless adjacent).
#[inline]
pub const fn neighbours4(m: u128) -> u128 {
    (((m << 1) & NOT_COL0) | ((m >> 1) & NOT_COL9) | (m << SIZE) | (m >> SIZE)) & Bitboard::FULL.0
}

/// `m` grown by one cell in all eight directions.
#[inline]
pub const fn grow8(m: u128) -> u128 {
    let row = m | ((m << 1) & NOT_COL0) | ((m >> 1) & NOT_COL9);
    (row | row << SIZE | row >> SIZE) & Bitboard::FULL.0
}

/// Iterator over set bit indices of a mask.
#[derive(Clone, Copy)]
pub struct Bits(pub u128);

impl Iterator for Bits {
    type Item = usize;
    #[inline]
    fn next(&mut self) -> Option<usize> {
        if self.0 == 0 {
            return None;
        }
        let i = self.0.trailing_zeros() as usize;
        self.0 &= self.0 - 1;
        Some(i)
    }
}

/// Classic coordinates: row letter A-J, column number 1-10 (`A1` is top-left).
pub fn cell_name(index: usize) -> String {
    format!(
        "{}{}",
        (b'A' + (index / SIZE) as u8) as char,
        index % SIZE + 1
    )
}

/// Parses `B7`, `b7`, `J10`; also accepts the column first (`7B`).
pub fn parse_cell(s: &str) -> Option<usize> {
    let s = s.trim().to_ascii_uppercase();
    if !s.is_ascii() {
        return None;
    }
    let letter_first = s.chars().next()?.is_ascii_alphabetic();
    let (l, n) = if letter_first {
        s.split_at(1)
    } else {
        s.split_at(s.len() - 1)
    };
    let (l, n) = if letter_first { (l, n) } else { (n, l) };
    let row = l.chars().next()?;
    let col: usize = n.parse().ok()?;
    if !('A'..='J').contains(&row) || !(1..=SIZE).contains(&col) {
        return None;
    }
    Some((row as u8 - b'A') as usize * SIZE + col - 1)
}

impl BitOr for Bitboard {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Bitboard {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Bitboard {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for Bitboard {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl Not for Bitboard {
    type Output = Self;
    /// Complement within the 100 board cells (the 28 spare bits stay zero).
    #[inline]
    fn not(self) -> Self {
        Self(!self.0 & Self::FULL.0)
    }
}

impl fmt::Display for Bitboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in 0..SIZE {
            for col in 0..SIZE {
                let c = if self.contains(row, col) { '#' } else { '.' };
                write!(f, "{c}")?;
                if col + 1 < SIZE {
                    write!(f, " ")?;
                }
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_has_100_cells() {
        assert_eq!(Bitboard::FULL.count(), 100);
        assert!((!Bitboard::FULL).is_empty());
    }

    #[test]
    fn cell_roundtrip() {
        let b = Bitboard::cell(3, 7);
        assert!(b.contains(3, 7));
        assert!(!b.contains(7, 3));
        assert_eq!(b.count(), 1);
    }

    #[test]
    fn iterates_bits() {
        let b = Bitboard::cell(0, 0) | Bitboard::cell(9, 9) | Bitboard::cell(6, 4);
        assert_eq!(b.iter().collect::<Vec<_>>(), vec![0, 64, 99]);
    }

    #[test]
    fn neighbours_stay_on_board() {
        let corner = Bitboard::cell(0, 9).0;
        assert_eq!(
            neighbours4(corner),
            Bitboard::cell(0, 8).0 | Bitboard::cell(1, 9).0
        );
        assert_eq!(grow8(Bitboard::cell(5, 0).0).count_ones(), 6);
        assert_eq!(grow8(Bitboard::cell(5, 5).0).count_ones(), 9);
    }

    #[test]
    fn cell_names() {
        for i in 0..CELLS {
            assert_eq!(parse_cell(&cell_name(i)), Some(i));
        }
        assert_eq!(parse_cell("a1"), Some(0));
        assert_eq!(parse_cell("10J"), Some(99));
        assert_eq!(parse_cell("K1"), None);
        assert_eq!(parse_cell("A11"), None);
        assert_eq!(parse_cell(""), None);
    }
}
