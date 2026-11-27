//! Hunting the two-cell ship is a covering problem: every place it can
//! still lie must be shot at once, and a shot at either of its two cells
//! does. The fewest cells that reach every such place are a smallest vertex
//! cover of the graph whose edges are those places. A miss on a cell of such
//! a cover leaves a cover one cell smaller; a miss anywhere else leaves the
//! work as it was, and is wasted as far as that ship goes.
//!
//! On an open board the covers are the two colours of a checkerboard, so
//! this is the familiar advice to hunt on one colour; around misses, sunk
//! ships and the edges it says what the checkerboard cannot.

use crate::bitboard::{Bits, CELLS, SIZE};

/// The cells that lie in some smallest set of cells reaching every edge.
/// `across` and `down`: cells from which an edge runs to the next cell in
/// the row, and to the cell below.
pub fn cover_cells(across: u128, down: u128) -> u128 {
    // Neighbours per cell (at most four).
    let mut near = [[0u8; 4]; CELLS];
    let mut deg = [0usize; CELLS];
    let mut link = |a: usize, b: usize| {
        near[a][deg[a]] = b as u8;
        deg[a] += 1;
        near[b][deg[b]] = a as u8;
        deg[b] += 1;
    };
    for c in Bits(across) {
        link(c, c + 1);
    }
    for c in Bits(down) {
        link(c, c + SIZE);
    }
    const NONE: u8 = u8::MAX;
    let mut mate = [NONE; CELLS];
    // A largest matching, by augmenting paths from the cells of one colour.
    fn augment(
        v: usize,
        near: &[[u8; 4]; CELLS],
        deg: &[usize; CELLS],
        mate: &mut [u8; CELLS],
        seen: &mut u128,
    ) -> bool {
        for &w in &near[v][..deg[v]] {
            let w = w as usize;
            if *seen >> w & 1 == 1 {
                continue;
            }
            *seen |= 1 << w;
            if mate[w] == NONE || augment(mate[w] as usize, near, deg, mate, seen) {
                mate[w] = v as u8;
                mate[v] = w as u8;
                return true;
            }
        }
        false
    }
    for v in (0..CELLS).filter(|&v| (v / SIZE + v % SIZE).is_multiple_of(2) && deg[v] > 0) {
        let mut seen = 0u128;
        augment(v, &near, &deg, &mut mate, &mut seen);
    }
    // A cell lies in some smallest cover unless some largest matching
    // leaves it out: the cells reached from an unmatched one by a path that
    // alternates between edges outside and inside the matching.
    let mut out = 0u128;
    let mut stack: Vec<usize> = Vec::with_capacity(CELLS);
    for v in 0..CELLS {
        if deg[v] > 0 && mate[v] == NONE {
            out |= 1 << v;
            stack.push(v);
        }
    }
    while let Some(v) = stack.pop() {
        for &w in &near[v][..deg[v]] {
            let u = mate[w as usize];
            if u != NONE && out >> u & 1 == 0 {
                out |= 1 << u;
                stack.push(u as usize);
            }
        }
    }
    let mut cells = 0u128;
    for v in (0..CELLS).filter(|&v| deg[v] > 0) {
        cells |= 1 << v;
    }
    cells & !out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitboard::Bitboard;
    use crate::fleet::starts;
    use crate::rng::Rng;

    /// Size of a smallest cover of the pairs of neighbours inside `open`,
    /// by trying every set of cells.
    fn smallest(open: u128) -> u32 {
        let cells: Vec<usize> = Bits(open).collect();
        let (h, v) = starts(2, open);
        let edges: Vec<(usize, usize)> = Bits(h)
            .map(|c| (c, c + 1))
            .chain(Bits(v).map(|c| (c, c + SIZE)))
            .collect();
        (0u32..1 << cells.len())
            .filter(|set| {
                let has = |c: usize| set >> cells.iter().position(|&x| x == c).unwrap() & 1 == 1;
                edges.iter().all(|&(a, b)| has(a) || has(b))
            })
            .map(|set| set.count_ones())
            .min()
            .unwrap()
    }

    #[test]
    fn open_board_then_one_colour() {
        let full = Bitboard::FULL.0;
        let (h, v) = starts(2, full);
        assert_eq!(cover_cells(h, v), full);
        let open = full & !(1 << 44);
        let (h, v) = starts(2, open);
        let cover = cover_cells(h, v);
        assert_eq!(cover.count_ones(), 49);
        assert!(Bits(cover).all(|c| (c / SIZE + c % SIZE).is_multiple_of(2)));
    }

    #[test]
    fn cells_of_smallest_covers_match_brute_force() {
        let mut rng = Rng::new(9);
        for _ in 0..300 {
            // A few open cells in a corner of the board.
            let mut open = 0u128;
            for _ in 0..14 {
                let (r, c) = (rng.below(4) as usize, rng.below(5) as usize);
                open |= 1 << (r * SIZE + c);
            }
            let (h, v) = starts(2, open);
            let cover = cover_cells(h, v);
            let size = smallest(open);
            for c in Bits(open) {
                // In some smallest cover: without the cell, one fewer does.
                let helps = smallest(open & !(1 << c)) + 1 == size;
                assert_eq!(cover >> c & 1 == 1, helps, "cell {c} of {open:#x}");
            }
        }
    }
}
