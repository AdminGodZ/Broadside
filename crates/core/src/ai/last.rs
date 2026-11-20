//! The last ship, not yet hit: the best order to look for it, exactly.
//!
//! With one ship afloat and none of its cells hit, the sunk ships fill the
//! hit cells whatever the last ship does, so every placement of it in the
//! unshot cells is equally likely. Until the first hit every answer is a
//! miss, so a plan is just an order of cells. And once a cell is hit, at
//! most `2 * len` placements are left, lying in its row and column, and
//! what finishing them costs depends only on how far the unshot cells run
//! from the hit in the four directions: a small table. That leaves
//!
//! ```text
//! cost(open) = placements(open) + min over x [ cost(open - x) + finish(x) ]
//! ```
//!
//! counted in shots summed over all placements. Only the misses branch.
//! The search over it is cut by a bound: the cells of an order are distinct
//! and a cell never covers more placements later than it does now, and of
//! two cells that share a placement and cover equally many, the one shot
//! second covers fewer by then.

use crate::bitboard::{Bitboard, Bits, CELLS, SIZE};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Longest ship.
const LONGEST: usize = 5;
/// A cell lies on at most this many placements of one ship.
const COVER: usize = 2 * LONGEST;
/// Positions kept per thread before the table is started afresh.
const MEMORY: usize = 400_000;

/// What the search needs to know about ships of one length.
struct Shape {
    len: usize,
    /// Cells a ship lying across can start at.
    across: u128,
    /// Per cell: the starts of the ships across, and down, that hold it.
    starts_across: [u128; CELLS],
    starts_down: [u128; CELLS],
    /// Per cell: the other cells that can share a ship with it.
    near: [u128; CELLS],
    /// Shots to finish, summed over the placements through a cell just hit,
    /// by how many unshot cells follow it left, right, up and down (each at
    /// most `len - 1`).
    finish: Vec<u32>,
    /// `unit[j - 1]`: no cell on `c` placements costs less to finish than
    /// the first `c` of these together; they never decrease.
    unit: Vec<u32>,
}

impl Shape {
    fn new(len: usize) -> Self {
        let mut across = 0u128;
        let mut starts_across = [0u128; CELLS];
        let mut starts_down = [0u128; CELLS];
        let mut near = [0u128; CELLS];
        for c in 0..CELLS {
            let (row, col) = (c / SIZE, c % SIZE);
            if col + len <= SIZE {
                across |= 1 << c;
            }
            for i in 0..len {
                if col >= i {
                    starts_across[c] |= 1 << (c - i);
                }
                if row >= i {
                    starts_down[c] |= 1 << (c - i * SIZE);
                }
                if i > 0 {
                    if col >= i {
                        near[c] |= 1 << (c - i);
                    }
                    if col + i < SIZE {
                        near[c] |= 1 << (c + i);
                    }
                    if row >= i {
                        near[c] |= 1 << (c - i * SIZE);
                    }
                    if row + i < SIZE {
                        near[c] |= 1 << (c + i * SIZE);
                    }
                }
            }
        }
        // The finishing costs, and the least of them per number of
        // placements through the cell.
        let mut finish = vec![0; len.pow(4)];
        let mut least = [u32::MAX; COVER + 1];
        for (i, f) in finish.iter_mut().enumerate() {
            let runs = [i % len, i / len % len, i / len / len % len, i / len.pow(3)];
            let count = |a: usize, b: usize| (a + b + 2).saturating_sub(len);
            let n = count(runs[0], runs[1]) + count(runs[2], runs[3]);
            *f = finish_cost(len, runs);
            least[n] = least[n].min(*f);
        }
        let mut unit = Vec::new();
        let mut spent = 0;
        for j in 1..=2 * len {
            let u = (j..=2 * len)
                .filter(|&c| least[c] != u32::MAX)
                .map(|c| (least[c] - spent) / (c - j + 1) as u32)
                .min()
                .unwrap_or(u32::MAX / 4);
            unit.push(u);
            spent += u.min(1 << 20);
        }
        Self {
            len,
            across,
            starts_across,
            starts_down,
            near,
            finish,
            unit,
        }
    }

    /// Where ships can start, across and down, with these cells unshot.
    #[inline]
    fn starts(&self, open: u128) -> (u128, u128) {
        let (mut a, mut d) = (open & self.across, open);
        for i in 1..self.len {
            a &= open >> i;
            d &= open >> (i * SIZE);
        }
        (a, d)
    }

    /// The cells on at least one ship.
    #[inline]
    fn live(&self, (a, d): (u128, u128)) -> u128 {
        let mut out = 0;
        for i in 0..self.len {
            out |= a << i | d << (i * SIZE);
        }
        out
    }

    /// How many ships hold `x`.
    #[inline]
    fn cover(&self, (a, d): (u128, u128), x: usize) -> u32 {
        (a & self.starts_across[x]).count_ones() + (d & self.starts_down[x]).count_ones()
    }

    /// Shots to finish after a hit at `x`, summed over the ships holding it.
    fn finish(&self, open: u128, x: usize, cover: u32) -> u32 {
        if self.len == 2 {
            // The other half is one of `cover` cells, each as likely.
            return cover * (cover + 1) / 2;
        }
        let (row, col) = (x / SIZE, x % SIZE);
        let run = |step: isize, room: usize| {
            let mut n = 0;
            while n < room.min(self.len - 1)
                && open >> (x as isize + step * (n as isize + 1)) & 1 == 1
            {
                n += 1;
            }
            n
        };
        let (l, r) = (run(-1, col), run(1, SIZE - 1 - col));
        let (u, d) = (
            run(-(SIZE as isize), row),
            run(SIZE as isize, SIZE - 1 - row),
        );
        let len = self.len;
        self.finish[l + len * (r + len * (u + len * d))]
    }

    /// No order of shots at `open` costs less than this.
    fn bound(&self, starts: (u128, u128), live: u128) -> u32 {
        // Cells by how many ships hold them, and for each such number a
        // set of disjoint pairs of cells that share a ship.
        let mut cells = [0u128; COVER + 1];
        for x in Bits(live) {
            cells[self.cover(starts, x) as usize] |= 1 << x;
        }
        let mut pairs = [0u32; COVER + 1];
        for (t, &group) in cells.iter().enumerate().skip(1) {
            let mut free = group;
            while free != 0 {
                let x = free.trailing_zeros() as usize;
                free &= free - 1;
                let partner = Bits(free & self.near[x]).find(|&y| {
                    let (a, d) = starts;
                    a & self.starts_across[x] & self.starts_across[y] != 0
                        || d & self.starts_down[x] & self.starts_down[y] != 0
                });
                if let Some(y) = partner {
                    free &= !(1 << y);
                    pairs[t] += 1;
                }
            }
        }
        // rows[j]: how many shots can still find at least j ships.
        let mut rows = [0u32; COVER + 1];
        let mut above = 0;
        for t in (1..=COVER).rev() {
            above += cells[t].count_ones();
            rows[t] = above - pairs[t];
        }
        // The i-th shot's j-th ship costs i for the wait and unit[j] for
        // the finish: the cheapest of these, one per ship.
        let m = starts.0.count_ones() + starts.1.count_ones();
        let top = 2 * self.len;
        let within = |limit: u32| -> u32 {
            (1..=top)
                .map(|j| limit.saturating_sub(self.unit[j - 1]).min(rows[j]))
                .sum()
        };
        let (mut lo, mut hi) = (0, self.unit[top - 1].min(1 << 20) + CELLS as u32 + 1);
        if within(hi) < m {
            // Fewer than the ships there are: count what there is.
            lo = hi;
        }
        while lo < hi {
            let mid = (lo + hi) / 2;
            if within(mid) >= m {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let (mut total, mut taken) = (0, 0);
        for (&unit, &rows) in self.unit.iter().zip(&rows[1..]) {
            let n = (lo.saturating_sub(1)).saturating_sub(unit).min(rows);
            total += n * unit + n * (n + 1) / 2;
            taken += n;
        }
        total + m.saturating_sub(taken) * lo
    }
}

/// Shots to finish a ship of `len` after its first hit, summed over its
/// placements, when `runs` unshot cells follow the hit left, right, up and
/// down: every way of playing it out, the best taken.
fn finish_cost(len: usize, runs: [usize; 4]) -> u32 {
    // A placement as the set of its cells other than the hit one: bit
    // `offset + len - 1` across, the same above `2 * len` down.
    let mut ships: Vec<u32> = Vec::new();
    for (axis, (before, after)) in [(runs[0], runs[1]), (runs[2], runs[3])]
        .into_iter()
        .enumerate()
    {
        for start in -(before as isize)..=0 {
            if start + len as isize - 1 > after as isize {
                continue;
            }
            let mut m = 0u32;
            for o in start..start + len as isize {
                if o != 0 {
                    m |= 1 << (o + len as isize - 1 + (axis * 2 * len) as isize);
                }
            }
            ships.push(m);
        }
    }
    fn go(
        ships: &[u32],
        left: u16,
        hit: u32,
        len: usize,
        memo: &mut HashMap<(u16, u32), u32>,
    ) -> u32 {
        let n = left.count_ones();
        if n == 0 {
            return 0;
        }
        if n == 1 {
            return len as u32 - 1 - hit.count_ones();
        }
        if let Some(&v) = memo.get(&(left, hit)) {
            return v;
        }
        let mut cells = 0u32;
        for (i, &s) in ships.iter().enumerate() {
            if left >> i & 1 == 1 {
                cells |= s;
            }
        }
        cells &= !hit;
        let mut best = u32::MAX;
        while cells != 0 {
            let y = cells.isolate_lowest_one();
            cells ^= y;
            let mut with = 0u16;
            for (i, &s) in ships.iter().enumerate() {
                if left >> i & 1 == 1 && s & y != 0 {
                    with |= 1 << i;
                }
            }
            let cost =
                n + go(ships, with, hit | y, len, memo) + go(ships, left & !with, hit, len, memo);
            best = best.min(cost);
        }
        memo.insert((left, hit), best);
        best
    }
    let all = ((1u32 << ships.len()) - 1) as u16;
    go(&ships, all, 0, len, &mut HashMap::new())
}

fn shape(len: usize) -> &'static Shape {
    static SHAPES: [OnceLock<Shape>; LONGEST + 1] = [const { OnceLock::new() }; LONGEST + 1];
    SHAPES[len].get_or_init(|| Shape::new(len))
}

/// A position's cost, or a floor under it that was enough at the time.
type Memo = HashMap<u128, (u32, bool)>;

thread_local! {
    /// Per ship length: what positions cost. True whatever the game, so it
    /// is kept from move to move and from game to game.
    static MEMO: RefCell<[Memo; LONGEST + 1]> = RefCell::default();
}

struct Search<'a> {
    shape: &'a Shape,
    memo: &'a mut Memo,
    /// Positions that may still be opened.
    budget: u64,
}

impl Search<'_> {
    /// The cells worth a shot, most promising first, each with the ships
    /// it finds and what finishing them costs.
    fn shots(&self, open: u128, starts: (u128, u128)) -> Vec<(usize, u32, u32)> {
        let mut out: Vec<_> = Bits(self.shape.live(starts))
            .map(|x| {
                let c = self.shape.cover(starts, x);
                (x, c, self.shape.finish(open, x, c))
            })
            .collect();
        out.sort_by_key(|&(x, c, f)| (std::cmp::Reverse(c), f, x));
        out
    }

    /// The least total of shots over all ships in `open`, if below `limit`;
    /// otherwise some floor under it that is at least `limit`. `None` when
    /// the budget ran out.
    fn cost(&mut self, open: u128, limit: u32) -> Option<u32> {
        let starts = self.shape.starts(open);
        let open = self.shape.live(starts);
        if open == 0 {
            return Some(0);
        }
        let known = self.memo.get(&open).copied();
        match known {
            Some((v, true)) => return Some(v),
            Some((v, false)) if v >= limit => return Some(v),
            _ => {}
        }
        let floor = self.shape.bound(starts, open);
        if floor >= limit {
            self.memo.insert(open, (floor, false));
            return Some(floor);
        }
        if self.budget == 0 {
            return None;
        }
        self.budget -= 1;
        let m = starts.0.count_ones() + starts.1.count_ones();
        let (mut best, mut found, mut lowest) = (limit, false, u32::MAX);
        for (x, _, finish) in self.shots(open, starts) {
            let first = m + finish;
            if first >= best {
                lowest = lowest.min(first);
                continue;
            }
            let q = first + self.cost(open & !(1 << x), best - first)?;
            if q < best {
                (best, found) = (q, true);
            } else {
                lowest = lowest.min(q);
            }
        }
        let value = if found { best } else { lowest };
        self.memo.insert(open, (value, found));
        Some(value)
    }

    /// Always the cell on most ships: a plan to beat, and its first shot.
    fn greedy(&self, mut open: u128) -> (u32, usize) {
        let (mut total, mut first) = (0, None);
        loop {
            let starts = self.shape.starts(open);
            let m = starts.0.count_ones() + starts.1.count_ones();
            let Some(&(x, _, finish)) = self.shots(open, starts).first() else {
                return (total, first.unwrap_or(0));
            };
            total += m + finish;
            first.get_or_insert(x);
            open &= !(1 << x);
        }
    }
}

/// The best first shot at the last ship.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Order {
    pub shot: usize,
    /// Shots still needed under best play, summed over all placements.
    pub total: u32,
    pub placements: u32,
    /// Positions the search opened.
    pub opened: u64,
}

/// The best shot when a ship of `len` cells is the only one afloat, none of
/// its cells is hit, and `open` are the unshot cells. `None` if there is no
/// room for the ship, or if the answer is not found by opening `budget`
/// positions.
pub fn best(open: Bitboard, len: usize, budget: u64) -> Option<Order> {
    let shape = shape(len);
    let open = shape.live(shape.starts(open.0 & Bitboard::FULL.0));
    if open == 0 {
        return None;
    }
    MEMO.with_borrow_mut(|memos| {
        let memo = &mut memos[len];
        if memo.len() > MEMORY {
            memo.clear();
        }
        let mut search = Search {
            shape,
            memo,
            budget,
        };
        let starts = shape.starts(open);
        let m = starts.0.count_ones() + starts.1.count_ones();
        let (mut best, mut shot) = search.greedy(open);
        for (x, _, finish) in search.shots(open, starts) {
            let first = m + finish;
            if first >= best {
                continue;
            }
            let q = first + search.cost(open & !(1 << x), best - first)?;
            if q < best {
                (best, shot) = (q, x);
            }
        }
        Some(Order {
            shot,
            total: best,
            placements: m,
            opened: budget - search.budget,
        })
    })
}

/// The total of shots over all placements when `shot` is fired first and
/// the best order follows (see `best`).
pub fn after(open: Bitboard, len: usize, shot: usize, budget: u64) -> Option<u32> {
    let shape = shape(len);
    let starts = shape.starts(open.0 & Bitboard::FULL.0);
    let open = shape.live(starts);
    let m = starts.0.count_ones() + starts.1.count_ones();
    let rest = |left: u128| {
        if shape.live(shape.starts(left)) == 0 {
            return Some(0);
        }
        best(Bitboard(left), len, budget).map(|o| o.total)
    };
    if open >> shot & 1 == 0 {
        // A wasted shot, in every placement.
        return Some(m + rest(open)?);
    }
    let finish = shape.finish(open, shot, shape.cover(starts, shot));
    Some(m + finish + rest(open & !(1 << shot))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    /// The recurrence with nothing cut and nothing remembered across calls.
    fn plain(shape: &Shape, open: u128, memo: &mut HashMap<u128, u32>) -> u32 {
        let starts = shape.starts(open);
        let open = shape.live(starts);
        if open == 0 {
            return 0;
        }
        if let Some(&v) = memo.get(&open) {
            return v;
        }
        let m = starts.0.count_ones() + starts.1.count_ones();
        let best = Bits(open)
            .map(|x| {
                let c = shape.cover(starts, x);
                m + shape.finish(open, x, c) + plain(shape, open & !(1 << x), memo)
            })
            .min()
            .unwrap();
        memo.insert(open, best);
        best
    }

    fn rectangle(rows: usize, cols: usize) -> Bitboard {
        let mut b = 0u128;
        for r in 0..rows {
            for c in 0..cols {
                b |= 1 << (r * SIZE + c);
            }
        }
        Bitboard(b)
    }

    #[test]
    fn finishing_a_destroyer_is_trying_its_neighbours_in_turn() {
        for i in 0..16 {
            let runs = [i & 1, i >> 1 & 1, i >> 2 & 1, i >> 3];
            let n = runs.iter().sum::<usize>() as u32;
            assert_eq!(finish_cost(2, runs), n * (n + 1) / 2);
        }
        assert_eq!(shape(2).unit, [1, 2, 3, 4]);
    }

    #[test]
    fn finishing_costs_by_hand() {
        // A cruiser hit in the middle of five cells in a row, nothing above
        // or below: three placements. A shot next to the hit finds two of
        // them, and one more tells those apart: 2 and 3 shots. The third
        // takes the miss and its own two cells: 3 shots.
        assert_eq!(finish_cost(3, [2, 2, 0, 0]), 8);
        // One placement: its two other cells.
        assert_eq!(finish_cost(3, [2, 0, 0, 0]), 2);
        assert_eq!(finish_cost(5, [0, 4, 0, 0]), 4);
    }

    #[test]
    fn one_destroyer_on_an_empty_rectangle() {
        // Total shots over all placements under best play.
        for (rows, cols, total) in [
            (2, 2, 12),
            (2, 3, 25),
            (3, 4, 87),
            (4, 4, 146),
            (4, 5, 218),
            (6, 6, 648),
        ] {
            let found = best(rectangle(rows, cols), 2, 2_000_000).map(|o| o.total);
            assert_eq!(found, Some(total), "{rows}x{cols}");
        }
    }

    #[test]
    fn the_bound_never_exceeds_the_cost_and_the_search_agrees_with_plain_recursion() {
        let mut rng = Rng::new(11);
        for len in 2..=5 {
            let shape = shape(len);
            let mut memo = HashMap::new();
            for _ in 0..60 {
                // A handful of cells in a corner of the board.
                let (rows, cols) = (3 + rng.below(2) as usize, 4 + rng.below(2) as usize);
                let mut open = rectangle(rows.max(len.min(5)), cols.max(len.min(5))).0;
                for _ in 0..rng.below(8) {
                    open &= !(1u128 << (rng.below(5) as usize * SIZE + rng.below(5) as usize));
                }
                let want = plain(shape, open, &mut memo);
                let starts = shape.starts(open);
                assert!(
                    shape.bound(starts, shape.live(starts)) <= want,
                    "len {len} {open:x}"
                );
                let got = best(Bitboard(open), len, 10_000_000).map_or(0, |o| o.total);
                assert_eq!(got, want, "len {len} {open:x}");
            }
        }
    }
}
