//! Exact posterior over fleet layouts, given everything the shooter knows.
//!
//! A layout is consistent with the knowledge when
//! - no ship covers a miss and every hit is covered,
//! - each sunk ship contains its finishing shot and lies inside the hits that
//!   stood when it sank,
//! - no ship still afloat is fully hit.
//!
//! Under a uniform prior every consistent layout is equally likely, so the
//! posterior is a count. We count exactly, in three layers:
//! 1. enumerate placements of sunk ships (few candidates each);
//! 2. exact-cover search: take the lowest uncovered hit and branch on which
//!    afloat ship covers it, until every hit is accounted for;
//! 3. the ships left over touch no hit: they are "free" in the unexplored
//!    region. We loop over all but two of them and count the last pair in
//!    closed form with popcounts (pairs = n_a * n_b - overlapping pairs).
//!
//! Counts per (ship, placement) fall out of the loops for free, and cell
//! probabilities follow from those.

use crate::bitboard::{Bits, CELLS, SIZE};
use crate::fleet::{LENGTHS, SEGMENTS, SHIPS, SLOTS, THROUGH, starts};
use crate::game::Knowledge;
use crate::symmetry::{self, CELL_MAP, SLOT_MAP};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

/// Positions worth caching: early ones, which recur across games and are the
/// slowest to count.
const CACHE_MIN_LAYOUTS: u64 = 100_000_000;
const CACHE_MAX_SHOTS: u32 = 24;
const CACHE_CAPACITY: usize = 10_000;

fn cache() -> &'static Mutex<HashMap<Knowledge, Arc<Posterior>>> {
    static CACHE: OnceLock<Mutex<HashMap<Knowledge, Arc<Posterior>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub type Weights = [[u64; SLOTS]; SHIPS];

#[derive(Clone)]
pub struct Posterior {
    /// Number of layouts consistent with the knowledge.
    pub layouts: u64,
    /// `ships[s][slot]`: consistent layouts with ship `s` at placement `slot`.
    pub ships: Box<Weights>,
    /// `cells[i]`: consistent layouts with a ship on cell `i`.
    pub cells: [u64; CELLS],
}

impl Posterior {
    pub fn compute(k: &Knowledge) -> Self {
        Self::compute_with(k, 1)
    }

    /// Uses up to `threads` threads for the expensive counting loops.
    pub fn compute_with(k: &Knowledge, threads: usize) -> Self {
        Self::compute_fixed(k, &[], threads)
    }

    /// Like `compute_with`, but shares results for early positions (up to
    /// symmetry) through a process-wide cache. Results are identical.
    pub fn cached(k: &Knowledge, threads: usize) -> Self {
        if k.shots_fired() > CACHE_MAX_SHOTS {
            return Self::compute_with(k, threads);
        }
        let (canon, t) = symmetry::canonical(k);
        let hit = cache().lock().unwrap().get(&canon).cloned();
        let p = match hit {
            Some(p) => p,
            None => {
                let p = Arc::new(Self::compute_with(&canon, threads));
                if p.layouts >= CACHE_MIN_LAYOUTS {
                    let mut c = cache().lock().unwrap();
                    if c.len() < CACHE_CAPACITY {
                        c.insert(canon, p.clone());
                    }
                }
                p
            }
        };
        p.untransform(t)
    }

    /// Maps a posterior of the image under transform `t` back to the original.
    fn untransform(&self, t: usize) -> Self {
        let mut out = Posterior {
            layouts: self.layouts,
            ships: Box::new([[0; SLOTS]; SHIPS]),
            cells: [0; CELLS],
        };
        for (c, &img) in CELL_MAP[t].iter().enumerate() {
            out.cells[c] = self.cells[img as usize];
        }
        for (ship, &len) in LENGTHS.iter().enumerate() {
            for slot in 0..SLOTS {
                if SEGMENTS[len][slot] != 0 {
                    out.ships[ship][slot] = self.ships[ship][SLOT_MAP[t][len][slot] as usize];
                }
            }
        }
        out
    }

    /// Posterior with some ships additionally pinned to given placements.
    pub fn compute_fixed(k: &Knowledge, fixed: &[(usize, u8)], threads: usize) -> Self {
        let mut s = Solver {
            misses: k.misses.0,
            hits: k.hits.0,
            threads: threads.max(1),
            w: Box::new([[0; SLOTS]; SHIPS]),
        };
        let sunk: Vec<(usize, Vec<u8>)> = (0..SHIPS)
            .filter_map(|ship| {
                let len = LENGTHS[ship];
                if let Some(&(_, slot)) = fixed.iter().find(|f| f.0 == ship) {
                    let m = SEGMENTS[len][slot as usize];
                    let ok = m != 0
                        && m & k.misses.0 == 0
                        && match k.sunk[ship] {
                            Some(sk) => m >> sk.cell & 1 == 1 && m & !sk.hits.0 == 0,
                            None => m & !k.hits.0 != 0,
                        };
                    return Some((ship, if ok { vec![slot] } else { vec![] }));
                }
                let sk = k.sunk[ship]?;
                let t = THROUGH[len][sk.cell as usize];
                let cands = t.slots[..t.n as usize]
                    .iter()
                    .copied()
                    .filter(|&slot| SEGMENTS[len][slot as usize] & !sk.hits.0 == 0)
                    .collect();
                Some((ship, cands))
            })
            .collect();
        let afloat = fixed.iter().fold(k.afloat_mask(), |m, f| m & !(1 << f.0));
        let layouts = s.sunk(&sunk, 0, afloat);

        let mut cells = [0u64; CELLS];
        for (ship, w) in s.w.iter().enumerate() {
            for (slot, &n) in w.iter().enumerate() {
                if n != 0 {
                    for c in Bits(SEGMENTS[LENGTHS[ship]][slot]) {
                        cells[c] += n;
                    }
                }
            }
        }
        Posterior {
            layouts,
            ships: s.w,
            cells,
        }
    }

    /// Draws placements for every ship still afloat, exactly from the
    /// posterior (one conditional computation per ship). Index by ship.
    pub fn sample_afloat(k: &Knowledge, rng: &mut crate::rng::Rng) -> [Option<u8>; SHIPS] {
        let mut fixed: Vec<(usize, u8)> = Vec::new();
        let mut out = [None; SHIPS];
        for ship in Bits(k.afloat_mask() as u128) {
            let p = Self::compute_fixed(k, &fixed, 1);
            let w = &p.ships[ship];
            let mut r = rng.below(p.layouts);
            let slot = w.iter().position(|&x| {
                if r < x {
                    true
                } else {
                    r -= x;
                    false
                }
            });
            let slot = slot.expect("weights of each ship sum to the layout count") as u8;
            fixed.push((ship, slot));
            out[ship] = Some(slot);
        }
        out
    }

    /// `[s][cell]`: consistent layouts with ship `s` on `cell`.
    pub fn ship_cells(&self) -> [[u64; CELLS]; SHIPS] {
        let mut out = [[0u64; CELLS]; SHIPS];
        for (ship, w) in self.ships.iter().enumerate() {
            for (slot, &n) in w.iter().enumerate() {
                if n != 0 {
                    for c in Bits(SEGMENTS[LENGTHS[ship]][slot]) {
                        out[ship][c] += n;
                    }
                }
            }
        }
        out
    }

    /// Probability that `cell` holds a ship.
    pub fn prob(&self, cell: usize) -> f64 {
        if self.layouts == 0 {
            0.0
        } else {
            self.cells[cell] as f64 / self.layouts as f64
        }
    }
}

struct Solver {
    misses: u128,
    hits: u128,
    threads: usize,
    w: Box<Weights>,
}

impl Solver {
    fn sunk(&mut self, list: &[(usize, Vec<u8>)], placed: u128, afloat: u8) -> u64 {
        let Some(((ship, cands), rest)) = list.split_first() else {
            return self.cover(placed, self.hits & !placed, afloat);
        };
        let mut z = 0;
        for &slot in cands {
            let m = SEGMENTS[LENGTHS[*ship]][slot as usize];
            if m & placed != 0 {
                continue;
            }
            let c = self.sunk(rest, placed | m, afloat);
            self.w[*ship][slot as usize] += c;
            z += c;
        }
        z
    }

    /// Assigns the uncovered hits to afloat ships, then counts the rest.
    fn cover(&mut self, placed: u128, uncovered: u128, afloat: u8) -> u64 {
        if uncovered == 0 {
            return self.free(!(self.misses | placed) & crate::Bitboard::FULL.0, afloat);
        }
        // Prune: the afloat ships must have room for every uncovered hit.
        let room: usize = Bits(afloat as u128).map(|s| LENGTHS[s]).sum();
        if room < uncovered.count_ones() as usize {
            return 0;
        }
        let h = uncovered.trailing_zeros() as usize;
        let blocked = self.misses | placed;
        let mut z = 0;
        for ship in Bits(afloat as u128) {
            let len = LENGTHS[ship];
            let t = THROUGH[len][h];
            for &slot in &t.slots[..t.n as usize] {
                let m = SEGMENTS[len][slot as usize];
                if m & blocked != 0 || m & !self.hits == 0 {
                    continue;
                }
                let c = self.cover(placed | m, uncovered & !m, afloat & !(1 << ship));
                self.w[ship][slot as usize] += c;
                z += c;
            }
        }
        z
    }

    /// Ships in `afloat` placed anywhere inside `free`, touching no hit.
    fn free(&mut self, free: u128, afloat: u8) -> u64 {
        let ships: Vec<usize> = Bits(afloat as u128).collect();
        let k = ships.len();
        if k == 0 {
            return 1;
        }
        let lens: Vec<usize> = ships.iter().map(|&s| LENGTHS[s]).collect();
        let n: Vec<u64> = lens
            .iter()
            .map(|&l| {
                let (h, v) = starts(l, free);
                (h.count_ones() + v.count_ones()) as u64
            })
            .collect();
        if n.contains(&0) {
            return 0;
        }

        // Pick runs (choice of inner ships) until every length is an outer
        // ship of some run; outer loops are where per-placement counts come from.
        let inner_size = if k >= 3 { 2 } else { k - 1 };
        let mut runs: Vec<Vec<usize>> = Vec::new(); // inner indices into `ships`
        let mut covered = 0u8; // by length
        let all: u8 = lens.iter().fold(0, |m, &l| m | 1 << l);
        while covered != all {
            let mut best: Option<(f64, Vec<usize>)> = None;
            for inner in subsets(k, inner_size) {
                let new = (0..k)
                    .filter(|i| !inner.contains(i))
                    .fold(0u8, |m, i| m | 1 << lens[i])
                    & !covered;
                if new == 0 {
                    continue;
                }
                let outer_work: f64 = (0..k)
                    .filter(|i| !inner.contains(i))
                    .map(|i| n[i] as f64)
                    .product();
                let il: Vec<usize> = inner.iter().map(|&i| lens[i]).collect();
                let score = outer_work * inner_cost(&il) / new.count_ones() as f64;
                if best.as_ref().is_none_or(|(b, _)| score < *b) {
                    best = Some((score, inner));
                }
            }
            let (_, inner) = best.expect("some run always covers a new length");
            covered |= (0..k)
                .filter(|i| !inner.contains(i))
                .fold(0u8, |m, i| m | 1 << lens[i]);
            runs.push(inner);
        }

        let mut z_total = None;
        let mut done = 0u8;
        for inner in runs {
            let outer: Vec<usize> = (0..k).filter(|i| !inner.contains(i)).collect();
            let outer_lens: Vec<usize> = outer.iter().map(|&i| lens[i]).collect();
            let inner_lens: Vec<usize> = inner.iter().map(|&i| lens[i]).collect();
            let work: f64 =
                outer.iter().map(|&i| n[i] as f64).product::<f64>() * inner_cost(&inner_lens);
            let threads = if work > 2e6 { self.threads } else { 1 };
            let mut local = vec![[0u64; SLOTS]; outer.len()];
            let z = exec_parallel(&outer_lens, &inner_lens, free, &mut local, threads);
            debug_assert!(z_total.is_none_or(|t| t == z));
            z_total = Some(z);
            for (pos, &i) in outer.iter().enumerate() {
                let len = lens[i];
                if done >> len & 1 == 1 {
                    continue;
                }
                done |= 1 << len;
                for &ship in ships.iter().filter(|&&s| LENGTHS[s] == len) {
                    for (acc, &x) in self.w[ship].iter_mut().zip(local[pos].iter()) {
                        *acc += x;
                    }
                }
            }
        }
        z_total.unwrap_or(1)
    }
}

/// All size-`m` subsets of `0..k`, as sorted index lists.
fn subsets(k: usize, m: usize) -> Vec<Vec<usize>> {
    (0u32..1 << k)
        .filter(|b| b.count_ones() as usize == m)
        .map(|b| (0..k).filter(|i| b >> i & 1 == 1).collect())
        .collect()
}

/// Relative cost of the closed-form count for the inner ships.
fn inner_cost(lens: &[usize]) -> f64 {
    match lens {
        [a, b] => (2 * (a + b - 1) + 2 * a * b + 6) as f64,
        _ => 2.0,
    }
}

/// Runs `exec` over `threads` disjoint shares of the outermost loop and sums.
fn exec_parallel(
    outer: &[usize],
    inner: &[usize],
    free: u128,
    w: &mut [[u64; SLOTS]],
    threads: usize,
) -> u64 {
    if threads <= 1 || outer.is_empty() {
        return exec(outer, inner, free, w, (0, 1));
    }
    let results: Vec<(u64, Vec<[u64; SLOTS]>)> = thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let mut local = vec![[0u64; SLOTS]; outer.len()];
                    let z = exec(outer, inner, free, &mut local, (t, threads));
                    (z, local)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut z = 0;
    for (zt, local) in results {
        z += zt;
        for (acc, l) in w.iter_mut().zip(local) {
            for (a, x) in acc.iter_mut().zip(l) {
                *a += x;
            }
        }
    }
    z
}

/// Counts layouts of `outer` ships (looped, outermost first) plus `inner`
/// ships (closed form) inside `free`, adding per-placement counts of each outer
/// ship to `w`. `share = (i, n)`: only every n-th outermost placement, from i.
fn exec(
    outer: &[usize],
    inner: &[usize],
    free: u128,
    w: &mut [[u64; SLOTS]],
    share: (usize, usize),
) -> u64 {
    macro_rules! with_inner {
        ($f:expr) => {
            match outer.len() {
                0 => $f(free),
                1 => run1(outer, free, $f, w, share),
                2 => run2(outer, free, $f, w, share),
                3 => run3(outer, free, $f, w, share),
                _ => unreachable!("at most three outer ships"),
            }
        };
    }
    macro_rules! pair {
        ($($a:literal $b:literal),*) => {
            match (inner[0], inner[1]) {
                $(($a, $b) => with_inner!(pair_count::<$a, $b>),)*
                _ => unreachable!("ship lengths are 2..=5"),
            }
        };
    }
    match inner.len() {
        0 => with_inner!(|_| 1),
        1 => match inner[0] {
            2 => with_inner!(count::<2>),
            3 => with_inner!(count::<3>),
            4 => with_inner!(count::<4>),
            5 => with_inner!(count::<5>),
            _ => unreachable!("ship lengths are 2..=5"),
        },
        _ => pair!(5 5, 5 4, 5 3, 5 2, 4 5, 4 4, 4 3, 4 2, 3 5, 3 4, 3 3, 3 2, 2 5, 2 4, 2 3, 2 2),
    }
}

/// Calls `f(slot, mask)` for every `len`-ship placement inside `free`.
#[inline(always)]
fn each(len: usize, free: u128, mut f: impl FnMut(usize, u128)) {
    let (h, v) = starts(len, free);
    let seg = &SEGMENTS[len];
    for start in Bits(h) {
        f(2 * start, seg[2 * start]);
    }
    for start in Bits(v) {
        f(2 * start + 1, seg[2 * start + 1]);
    }
}

/// `each` restricted to every `share.1`-th placement starting at `share.0`.
#[inline(always)]
fn each_shared(len: usize, free: u128, share: (usize, usize), mut f: impl FnMut(usize, u128)) {
    let mut i = 0;
    each(len, free, |slot, m| {
        if i % share.1 == share.0 {
            f(slot, m);
        }
        i += 1;
    });
}

fn run1(
    l: &[usize],
    free: u128,
    inner: impl Fn(u128) -> u64,
    w: &mut [[u64; SLOTS]],
    share: (usize, usize),
) -> u64 {
    let mut z = 0;
    each_shared(l[0], free, share, |s0, m0| {
        let c = inner(free & !m0);
        w[0][s0] += c;
        z += c;
    });
    z
}

fn run2(
    l: &[usize],
    free: u128,
    inner: impl Fn(u128) -> u64,
    w: &mut [[u64; SLOTS]],
    share: (usize, usize),
) -> u64 {
    let mut z = 0;
    each_shared(l[0], free, share, |s0, m0| {
        let f0 = free & !m0;
        let mut z0 = 0;
        each(l[1], f0, |s1, m1| {
            let c = inner(f0 & !m1);
            w[1][s1] += c;
            z0 += c;
        });
        w[0][s0] += z0;
        z += z0;
    });
    z
}

fn run3(
    l: &[usize],
    free: u128,
    inner: impl Fn(u128) -> u64,
    w: &mut [[u64; SLOTS]],
    share: (usize, usize),
) -> u64 {
    let mut z = 0;
    each_shared(l[0], free, share, |s0, m0| {
        let f0 = free & !m0;
        let mut z0 = 0;
        each(l[1], f0, |s1, m1| {
            let f1 = f0 & !m1;
            let mut z1 = 0;
            each(l[2], f1, |s2, m2| {
                let c = inner(f1 & !m2);
                w[2][s2] += c;
                z1 += c;
            });
            w[1][s1] += z1;
            z0 += z1;
        });
        w[0][s0] += z0;
        z += z0;
    });
    z
}

/// Placements of one `L`-ship inside `free`.
#[inline(always)]
fn count<const L: usize>(free: u128) -> u64 {
    let (h, v) = starts(L, free);
    (h.count_ones() + v.count_ones()) as u64
}

#[inline(always)]
fn pop(x: u128) -> u64 {
    x.count_ones() as u64
}

/// Disjoint placements of an `A`-ship and a `B`-ship inside `free`:
/// all pairs minus overlapping pairs. Parallel ships overlap when their starts
/// are within reach along the line; crossing ships share exactly one cell.
#[inline(always)]
fn pair_count<const A: usize, const B: usize>(free: u128) -> u64 {
    let (ha, va) = starts(A, free);
    let (hb, vb) = starts(B, free);
    let all = (pop(ha) + pop(va)) * (pop(hb) + pop(vb));
    let mut ov = 0;
    // Collinear: b starts d cells before (d < B) or after (d < A) a.
    for d in 1..B {
        ov += pop(ha & (hb << d)) + pop(va & (vb << (SIZE * d)));
    }
    for d in 0..A {
        ov += pop(ha & (hb >> d)) + pop(va & (vb >> (SIZE * d)));
    }
    // Crossing: cell a + i (along a) equals cell b + j (along b).
    for i in 0..A {
        for j in 0..B {
            ov += pop((ha << i) & (vb << (SIZE * j))) + pop((va << (SIZE * i)) & (hb << j));
        }
    }
    all - ov
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::FLEET;
    use crate::game::{Board, Layout, Outcome};
    use crate::rng::Rng;

    fn brute_pairs(a: usize, b: usize, free: u128) -> u64 {
        let pa: Vec<u128> = (0..SLOTS)
            .map(|s| SEGMENTS[a][s])
            .filter(|&m| m != 0 && m & !free == 0)
            .collect();
        let pb: Vec<u128> = (0..SLOTS)
            .map(|s| SEGMENTS[b][s])
            .filter(|&m| m != 0 && m & !free == 0)
            .collect();
        pa.iter()
            .map(|x| pb.iter().filter(|y| *x & **y == 0).count() as u64)
            .sum()
    }

    #[test]
    fn pair_count_matches_brute_force() {
        let mut rng = Rng::new(42);
        for trial in 0..60 {
            // Free masks from dense to sparse.
            let density = trial % 6;
            let mut free = crate::Bitboard::FULL.0;
            for _ in 0..density * 12 {
                free &= !(1u128 << rng.below(100));
            }
            macro_rules! check {
                ($($a:literal $b:literal),*) => {$(
                    assert_eq!(pair_count::<$a, $b>(free), brute_pairs($a, $b, free), "{} {}", $a, $b);
                )*};
            }
            check!(5 4, 5 3, 5 2, 4 3, 4 2, 3 3, 3 2, 2 2, 2 5, 5 5);
        }
    }

    /// Direct enumeration of the consistency definition, for cross-checking.
    fn naive(k: &Knowledge) -> (u64, [u64; CELLS]) {
        let cands: Vec<Vec<u128>> = (0..SHIPS)
            .map(|s| {
                let len = LENGTHS[s];
                (0..SLOTS)
                    .map(|slot| SEGMENTS[len][slot])
                    .filter(|&m| m != 0 && m & k.misses.0 == 0)
                    .filter(|&m| match k.sunk[s] {
                        Some(sk) => m >> sk.cell & 1 == 1 && m & !sk.hits.0 == 0,
                        None => m & !k.hits.0 != 0,
                    })
                    .collect()
            })
            .collect();
        let mut z = 0;
        let mut cells = [0u64; CELLS];
        let mut stack = [0u128; SHIPS];
        fn rec(
            i: usize,
            used: u128,
            cands: &[Vec<u128>],
            k: &Knowledge,
            stack: &mut [u128; SHIPS],
            z: &mut u64,
            cells: &mut [u64; CELLS],
        ) {
            if i == SHIPS {
                if k.hits.0 & !used == 0 {
                    *z += 1;
                    for c in Bits(used) {
                        cells[c] += 1;
                    }
                }
                return;
            }
            for &m in &cands[i] {
                if m & used == 0 {
                    stack[i] = m;
                    rec(i + 1, used | m, cands, k, stack, z, cells);
                }
            }
        }
        rec(0, 0, &cands, k, &mut stack, &mut z, &mut cells);
        (z, cells)
    }

    /// Plays a game with a shooter that loves shooting next to hits, so we get
    /// states with clustered, partly-sunk ships.
    fn random_state(rng: &mut Rng, shots: usize) -> Knowledge {
        let layout = Layout::random(rng);
        let mut board = Board::new(layout);
        let mut k = Knowledge::new();
        for _ in 0..shots {
            let unknown = k.unknown().0;
            if unknown == 0 || k.won() {
                break;
            }
            let h = k.hits.0;
            let near = ((h << 1) | (h >> 1) | (h << 10) | (h >> 10)) & unknown;
            let cell = if near != 0 && rng.below(4) != 0 {
                rng.pick_bit(near)
            } else {
                rng.pick_bit(unknown)
            };
            let o = board.fire(cell);
            k.record(cell, o);
        }
        k
    }

    #[test]
    fn matches_naive_enumeration() {
        let mut rng = Rng::new(7);
        let mut checked = 0;
        while checked < 150 {
            let shots = 45 + rng.below(40) as usize;
            let k = random_state(&mut rng, shots);
            let (z, cells) = naive(&k);
            let p = Posterior::compute(&k);
            assert!(z > 0, "the true layout is always consistent");
            assert_eq!(p.layouts, z, "{k:?}");
            assert_eq!(p.cells, cells, "{k:?}");
            checked += 1;
        }
    }

    #[test]
    fn parallel_matches_serial() {
        let mut rng = Rng::new(8);
        for _ in 0..5 {
            let k = random_state(&mut rng, 20);
            let a = Posterior::compute(&k);
            let b = Posterior::compute_with(&k, 4);
            assert_eq!(a.layouts, b.layouts);
            assert_eq!(a.cells, b.cells);
        }
    }

    #[test]
    fn cached_matches_direct_under_symmetry() {
        let mut rng = Rng::new(21);
        for _ in 0..20 {
            let k = random_state(&mut rng, 30);
            for t in 0..crate::symmetry::TRANSFORMS {
                let img = crate::symmetry::map_knowledge(t, &k);
                let a = Posterior::compute(&img);
                let b = Posterior::compute(&crate::symmetry::canonical(&img).0)
                    .untransform(crate::symmetry::canonical(&img).1);
                assert_eq!(a.cells, b.cells);
                assert_eq!(a.ships, b.ships);
            }
        }
    }

    #[test]
    fn sunk_ship_cells_are_certain() {
        let mut rng = Rng::new(5);
        let layout = Layout::random(&mut rng);
        let mut board = Board::new(layout);
        let mut k = Knowledge::new();
        for c in layout.ship(FLEET[0]).iter() {
            let o = board.fire(c);
            k.record(c, o);
        }
        assert_eq!(k.sunk[0].map(|s| s.hits), Some(layout.ship(FLEET[0])));
        let p = Posterior::compute(&k);
        for c in layout.ship(FLEET[0]).iter() {
            assert_eq!(p.cells[c], p.layouts);
        }
        let _ = Outcome::Miss;
    }

    /// Slow in debug builds: `cargo test --release -- --ignored`
    #[test]
    #[ignore]
    fn empty_board_counts_every_layout() {
        let p = Posterior::compute_with(&Knowledge::new(), 8);
        assert_eq!(p.layouts, 30_093_975_536);
        // Each ship's placement counts sum to the total.
        for s in 0..SHIPS {
            assert_eq!(p.ships[s].iter().sum::<u64>(), p.layouts);
        }
        // Expected occupied cells is exactly 17.
        assert_eq!(p.cells.iter().sum::<u64>(), 17 * p.layouts);
    }
}
