//! Endgame search over explicit worlds.
//!
//! Late in the game the ships still afloat have few enough joint placements
//! to list them all. Each is a "world", weighted by the number of layouts
//! behind it (arrangements of the sunk ships) and by the opponent prior. A
//! shot's outcome depends only on the world, so every later position is just
//! the subset of worlds that agree with the shots fired since. A whole line of
//! play is then evaluated by splitting index lists: no further inference.
//!
//! Values are total shots to finish, summed over worlds by weight. With a
//! uniform prior the weights are integers and all arithmetic is exact.
//!
//! Three policies continue a line of play, strongest where it is affordable:
//! - optimal: small positions are solved outright (`Exact`);
//! - improved: medium positions try the likeliest cells and keep the best
//!   under greedy continuation;
//! - greedy: fire at the likeliest cell.

use super::engine::Search;
use super::placement::Tilt;
use crate::bitboard::{Bits, CELLS};
use crate::fleet::{LENGTHS, SEGMENTS, SHIPS};
use crate::game::Knowledge;
use crate::posterior::Posterior;
use crate::rng::Rng;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// One joint placement of the ships still afloat.
#[derive(Clone, Copy, Debug)]
pub struct World {
    /// Cells per ship; 0 for ships already sunk.
    pub ships: [u128; SHIPS],
    pub all: u128,
    pub weight: f64,
}

/// Every world consistent with `k` (whose posterior is `p`), or `None` if
/// there are more than `limit`.
pub fn worlds(
    k: &Knowledge,
    p: &Posterior,
    limit: usize,
    prior: Option<&Tilt>,
) -> Option<Vec<World>> {
    let mut out: Vec<World> = p
        .enumerate_afloat(k, limit)?
        .into_iter()
        .map(|(slots, layouts)| {
            let mut ships = [0u128; SHIPS];
            for (s, slot) in slots.iter().enumerate() {
                if let Some(slot) = slot {
                    ships[s] = SEGMENTS[LENGTHS[s]][*slot as usize];
                }
            }
            World {
                ships,
                all: ships.iter().fold(0, |a, m| a | m),
                weight: layouts as f64,
            }
        })
        .collect();
    if let Some(t) = prior {
        // P(layout) ∝ Π e^θ over its cells. Every consistent layout holds
        // all the hit cells, so only a world's unshot cells set it apart.
        let unknown = k.unknown().0;
        let logs: Vec<f64> = out
            .iter()
            .map(|w| Bits(w.all & unknown).map(|c| t.theta[c]).sum())
            .collect();
        let top = logs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        for (w, l) in out.iter_mut().zip(logs) {
            w.weight *= (l - top).exp();
        }
        out.retain(|w| w.weight > 0.0);
    }
    Some(out)
}

/// Outcomes of a shot: miss, hit, or sinking one of the ships.
const OUTCOMES: usize = 2 + SHIPS;

#[inline]
fn outcome(w: &World, x: usize, hits: u128) -> usize {
    if w.all >> x & 1 == 0 {
        return 0;
    }
    let s = (0..SHIPS).find(|&s| w.ships[s] >> x & 1 == 1).unwrap();
    if w.ships[s] & !(hits | 1 << x) == 0 {
        2 + s
    } else {
        1
    }
}

/// The hits standing after a shot at `x` had outcome `o`.
#[inline]
fn advance(o: usize, x: usize, hits: u128) -> u128 {
    if o == 0 { hits } else { hits | 1 << x }
}

/// What a set of worlds has in common.
struct Summary {
    weight: f64,
    /// Unshot ship cells present in every world / in any world.
    and: u128,
    or: u128,
    /// Unshot cells that one and the same ship holds in every world.
    forced: u128,
}

impl Summary {
    fn of<'a>(worlds: impl Iterator<Item = &'a World>, hits: u128) -> Self {
        let (mut weight, mut and, mut or) = (0.0, u128::MAX, 0u128);
        let mut same = [u128::MAX; SHIPS];
        for w in worlds {
            weight += w.weight;
            and &= w.all;
            or |= w.all;
            for (a, m) in same.iter_mut().zip(w.ships) {
                *a &= m;
            }
        }
        Self {
            weight,
            and: and & !hits,
            or: or & !hits,
            forced: same.iter().fold(0, |a, m| a | m) & !hits,
        }
    }

    /// Every world has the same cells left: each remaining shot is a hit.
    fn settled(&self) -> Option<f64> {
        (self.and == self.or).then(|| self.weight * self.or.count_ones() as f64)
    }
}

/// Weight of the worlds holding each unshot cell.
fn occupancy<'a>(worlds: impl Iterator<Item = &'a World>, hits: u128) -> [f64; CELLS] {
    let mut occ = [0.0; CELLS];
    for w in worlds {
        for c in Bits(w.all & !hits) {
            occ[c] += w.weight;
        }
    }
    occ
}

/// Unshot cells some world holds, likeliest first (lowest cell on ties).
fn ranked(occ: &[f64; CELLS], or: u128) -> Vec<usize> {
    let mut cells: Vec<usize> = Bits(or).collect();
    cells.sort_by(|&a, &b| occ[b].total_cmp(&occ[a]));
    cells
}

/// Results by position: (tags of the surviving worlds XORed, hits standing).
type Table<V> = HashMap<(u128, u128), V, BuildHasherDefault<Mix>>;

pub struct Tree<'a> {
    worlds: &'a [World],
    /// The worlds' `all` and `weight`, packed: the loops over big positions
    /// touch nothing else.
    all: Vec<u128>,
    weight: Vec<f64>,
    cfg: Search,
    /// Positions the exact solver may still open for this decision.
    budget: u64,
    /// A random tag per world. Lines of play that fire the same shots in a
    /// different order reach the same position; keying positions by their
    /// worlds lets every table below recognize them.
    tags: Vec<u128>,
    /// Solved positions: optimal total shots and first shot.
    solved: Table<(f64, u8)>,
    /// `value` of positions, without and with improvement.
    values: [Table<f64>; 2],
    /// Scratch for `split`: the outcome per world.
    outcomes: Vec<u8>,
}

/// The exact solver's budget for a whole decision, in solves' worth.
const SOLVE_ATTEMPTS: u64 = 16;
/// Smaller positions are quicker to evaluate than to look up.
const TABLE_MIN: usize = 4;
/// Positions kept per table (about 50 bytes each).
const TABLE_MAX: usize = 2_000_000;

impl<'a> Tree<'a> {
    pub fn new(worlds: &'a [World], cfg: Search) -> Self {
        let mut rng = Rng::new(0x7A65);
        let tag = |_| (rng.next_u64() as u128) << 64 | rng.next_u64() as u128;
        Self {
            worlds,
            all: worlds.iter().map(|w| w.all).collect(),
            weight: worlds.iter().map(|w| w.weight).collect(),
            cfg,
            budget: cfg.solve_nodes.saturating_mul(SOLVE_ATTEMPTS),
            tags: (0..worlds.len()).map(tag).collect(),
            solved: Table::default(),
            values: Default::default(),
            outcomes: vec![0; worlds.len()],
        }
    }

    /// Sorts the worlds of `from` into `to` by the outcome of a shot at `x`;
    /// returns where each outcome's group starts.
    fn split(
        &mut self,
        x: usize,
        hits: u128,
        from: &[u32],
        to: &mut [u32],
    ) -> [usize; OUTCOMES + 1] {
        let mut cuts = [0usize; OUTCOMES + 1];
        for (&i, o) in from.iter().zip(self.outcomes.iter_mut()) {
            // Most worlds miss; only the others need a look at their ships.
            *o = if self.all[i as usize] >> x & 1 == 0 {
                0
            } else {
                outcome(&self.worlds[i as usize], x, hits) as u8
            };
            cuts[*o as usize + 1] += 1;
        }
        for o in 0..OUTCOMES {
            cuts[o + 1] += cuts[o];
        }
        let mut at = cuts;
        for (&i, &o) in from.iter().zip(&self.outcomes) {
            to[at[o as usize]] = i;
            at[o as usize] += 1;
        }
        cuts
    }

    /// `Summary` of the worlds in `idx`, and their key for the tables.
    fn scan(&self, idx: &[u32], hits: u128) -> (Summary, u128) {
        let (mut weight, mut and, mut or, mut key) = (0.0, u128::MAX, 0u128, 0u128);
        for &i in idx {
            let i = i as usize;
            weight += self.weight[i];
            and &= self.all[i];
            or |= self.all[i];
            key ^= self.tags[i];
        }
        // Only a cell every world holds can be one every world gives to
        // the same ship; mostly there is none, and the ships stay untouched.
        let mut forced = 0;
        if and & !hits != 0 {
            let mut same = [u128::MAX; SHIPS];
            for &i in idx {
                for (a, m) in same.iter_mut().zip(self.worlds[i as usize].ships) {
                    *a &= m;
                }
            }
            forced = same.iter().fold(0, |a, m| a | m) & !hits;
        }
        let s = Summary {
            weight,
            and: and & !hits,
            or: or & !hits,
            forced,
        };
        (s, key)
    }

    /// The shot to fire with `hits` standing, given every world: the cell.
    /// With `threads` above 1 the candidate shots are judged in parallel;
    /// the choice is the same.
    pub fn choose(&mut self, hits: u128, threads: usize) -> usize {
        let root: Vec<u32> = (0..self.worlds.len() as u32).collect();
        let s = Summary::of(self.worlds.iter(), hits);
        if s.settled().is_some() || s.forced != 0 {
            return (if s.forced != 0 { s.forced } else { s.or }).trailing_zeros() as usize;
        }
        if let Some((_, cell)) = self.solve(&root, hits, self.cfg.solve_worlds) {
            return cell;
        }
        let occ = occupancy(self.worlds.iter(), hits);
        let mut cands = ranked(&occ, s.or);
        cands.truncate(self.cfg.candidates.max(1));
        let values: Vec<f64> = if threads <= 1 || cands.len() == 1 {
            cands.iter().map(|&x| self.after(x, hits, &root)).collect()
        } else {
            // A tree of its own per thread: the tables are not shared, but
            // a position's value does not depend on them.
            let (worlds, cfg, next) = (self.worlds, self.cfg, AtomicUsize::new(0));
            let mut values = vec![0.0; cands.len()];
            thread::scope(|scope| {
                let workers: Vec<_> = (0..threads.min(cands.len()))
                    .map(|_| {
                        scope.spawn(|| {
                            let mut tree = Tree::new(worlds, cfg);
                            let mut done = Vec::new();
                            loop {
                                let i = next.fetch_add(1, Ordering::Relaxed);
                                let Some(&x) = cands.get(i) else {
                                    return done;
                                };
                                done.push((i, tree.after(x, hits, &root)));
                            }
                        })
                    })
                    .collect();
                for w in workers {
                    for (i, v) in w.join().unwrap() {
                        values[i] = v;
                    }
                }
            });
            values
        };
        let mut best = (f64::INFINITY, 0);
        for (&x, v) in cands.iter().zip(values) {
            // Leave the likeliest cell only for a real gain.
            if v < best.0 - 1e-9 * s.weight {
                best = (v, x);
            }
        }
        best.1
    }

    /// Total shots to finish after firing at `x`, less the shot itself.
    fn after(&mut self, x: usize, hits: u128, root: &[u32]) -> f64 {
        let n = root.len();
        let (mut y, mut z, mut t) = (vec![0u32; n], vec![0u32; n], vec![0u32; n]);
        let cuts = self.split(x, hits, root, &mut y);
        let mut v = 0.0;
        for o in 0..OUTCOMES {
            let r = cuts[o]..cuts[o + 1];
            if !r.is_empty() {
                let h = advance(o, x, hits);
                v += self.value(&mut y[r.clone()], &mut z[r.clone()], &mut t[r], h, true);
            }
        }
        v
    }

    /// Total shots to finish from the worlds in `x`, by the strongest policy
    /// the position's size allows. `y` and `z` are scratch of the same length;
    /// all three are overwritten. With `improve` off, play is greedy (or
    /// optimal where solved) and `z` stays untouched.
    fn value(
        &mut self,
        x: &mut [u32],
        y: &mut [u32],
        z: &mut [u32],
        hits: u128,
        improve: bool,
    ) -> f64 {
        let (s, key) = self.scan(x, hits);
        if let Some(v) = s.settled() {
            return v;
        }
        let key = (key, hits);
        if x.len() >= TABLE_MIN
            && let Some(&v) = self.values[improve as usize].get(&key)
        {
            return v;
        }
        let v = match self.solve(x, hits, self.cfg.leaf_worlds) {
            Some((v, _)) => v,
            None => self.play_on(x, y, z, hits, improve, &s),
        };
        if x.len() >= TABLE_MIN && self.values[improve as usize].len() < TABLE_MAX {
            self.values[improve as usize].insert(key, v);
        }
        v
    }

    /// `value` of a position that is neither settled nor solved.
    fn play_on(
        &mut self,
        x: &mut [u32],
        y: &mut [u32],
        z: &mut [u32],
        hits: u128,
        improve: bool,
        s: &Summary,
    ) -> f64 {
        let cell = if s.forced != 0 {
            s.forced.trailing_zeros() as usize
        } else {
            let mut occ = [0.0; CELLS];
            for &i in x.iter() {
                for c in Bits(self.all[i as usize] & !hits) {
                    occ[c] += self.weight[i as usize];
                }
            }
            if improve && x.len() <= self.cfg.deep_worlds {
                let mut best = (f64::INFINITY, 0);
                for c in ranked(&occ, s.or)
                    .into_iter()
                    .take(self.cfg.candidates.max(1))
                {
                    let cuts = self.split(c, hits, x, y);
                    let mut v = 0.0;
                    for o in 0..OUTCOMES {
                        let r = cuts[o]..cuts[o + 1];
                        if !r.is_empty() {
                            let h = advance(o, c, hits);
                            v += self.value(&mut y[r.clone()], &mut z[r], &mut [], h, false);
                        }
                    }
                    if v < best.0 - 1e-9 * s.weight {
                        best = (v, c);
                    }
                }
                best.1
            } else {
                // The likeliest cell, the lowest on ties: `ranked`'s first.
                Bits(s.or).fold(s.or.trailing_zeros() as usize, |best, c| {
                    if occ[c] > occ[best] { c } else { best }
                })
            }
        };
        let cuts = self.split(cell, hits, x, y);
        let mut total = s.weight;
        for o in 0..OUTCOMES {
            let r = cuts[o]..cuts[o + 1];
            if r.is_empty() {
                continue;
            }
            let h = advance(o, cell, hits);
            total += if improve {
                self.value(&mut y[r.clone()], &mut z[r.clone()], &mut x[r], h, true)
            } else {
                // Two buffers suffice, and the caller's third must survive.
                self.value(&mut y[r.clone()], &mut x[r], &mut [], h, false)
            };
        }
        total
    }

    /// Optimal total shots and first shot for a set of at most `limit`
    /// worlds, if the solver finishes within its budget.
    fn solve(&mut self, idx: &[u32], hits: u128, limit: usize) -> Option<(f64, usize)> {
        let n = idx.len();
        if n > limit.min(64) || self.budget == 0 {
            return None;
        }
        let mut e = Exact {
            worlds: idx.iter().map(|&i| self.worlds[i as usize]).collect(),
            tags: idx.iter().map(|&i| self.tags[i as usize]).collect(),
            memo: &mut self.solved,
            budget: self.budget.min(self.cfg.solve_nodes),
        };
        let granted = e.budget;
        let r = e.solve(u64::MAX >> (64 - n), hits);
        self.budget -= granted - e.budget;
        r.map(|(v, c)| (v, c as usize))
    }
}

/// Exact solver: the least expected shots from a small set of worlds, by
/// memoized search over (surviving worlds, hits standing).
struct Exact<'a> {
    worlds: Vec<World>,
    tags: Vec<u128>,
    memo: &'a mut Table<(f64, u8)>,
    /// Positions that may still be opened.
    budget: u64,
}

impl Exact<'_> {
    fn at(&self, set: u64) -> impl Iterator<Item = &World> {
        Bits(set as u128).map(|i| &self.worlds[i])
    }

    /// Least total shots over the worlds in `set`, and a first shot that
    /// achieves it. `None` when out of budget.
    fn solve(&mut self, set: u64, hits: u128) -> Option<(f64, u8)> {
        let s = Summary::of(self.at(set), hits);
        if let Some(v) = s.settled() {
            return Some((v, s.or.trailing_zeros() as u8));
        }
        let key = (Bits(set as u128).fold(0, |a, i| a ^ self.tags[i]), hits);
        if let Some(&v) = self.memo.get(&key) {
            return Some(v);
        }
        if self.budget == 0 {
            return None;
        }
        self.budget -= 1;
        let best = if s.forced != 0 {
            // A cell the same ship holds in every world costs nothing to take
            // first: the only thing it can change is when that ship's sinking
            // is announced, and that only gets earlier.
            let x = s.forced.trailing_zeros() as usize;
            (s.weight + self.after(set, x, hits, f64::INFINITY)?, x as u8)
        } else {
            let occ = occupancy(self.at(set), hits);
            let left = (self.worlds[set.trailing_zeros() as usize].all & !hits).count_ones();
            let mut best = (f64::INFINITY, 0u8);
            for x in ranked(&occ, s.or) {
                // Even if every later shot hits, this one misses in the worlds
                // without `x`. Cells come likeliest first, so once that floor
                // reaches the best line, no later cell can beat it.
                if s.weight * left as f64 + (s.weight - occ[x]) >= best.0 {
                    break;
                }
                let v = s.weight + self.after(set, x, hits, best.0 - s.weight)?;
                if v < best.0 {
                    best = (v, x as u8);
                }
            }
            best
        };
        self.memo.insert(key, best);
        Some(best)
    }

    /// Total over the outcomes of a shot at `x` of the solved sub-positions;
    /// infinity as soon as it cannot come in under `cap`.
    fn after(&mut self, set: u64, x: usize, hits: u128, cap: f64) -> Option<f64> {
        let mut groups = [0u64; OUTCOMES];
        for i in Bits(set as u128) {
            groups[outcome(&self.worlds[i], x, hits)] |= 1 << i;
        }
        // Floor per group: every cell still to hit takes a shot.
        let mut floors = [0.0; OUTCOMES];
        for o in (0..OUTCOMES).filter(|&o| groups[o] != 0) {
            let h = advance(o, x, hits);
            let left = (self.worlds[groups[o].trailing_zeros() as usize].all & !h).count_ones();
            let weight: f64 = self.at(groups[o]).map(|w| w.weight).sum();
            floors[o] = weight * left as f64;
        }
        let mut rest: f64 = floors.iter().sum();
        let mut total = 0.0;
        for o in (0..OUTCOMES).filter(|&o| groups[o] != 0) {
            if total + rest >= cap {
                return Some(f64::INFINITY);
            }
            rest -= floors[o];
            total += self.solve(groups[o], advance(o, x, hits))?.0;
        }
        Some(total)
    }
}

/// Cheap hasher for the solver's memo keys.
#[derive(Default)]
struct Mix(u64);

impl Hasher for Mix {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    fn write_u128(&mut self, x: u128) {
        self.write_u64(x as u64);
        self.write_u64((x >> 64) as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Shooter;
    use crate::ai::engine::{Broadside, rollout};
    use crate::fleet::Ship;
    use crate::game::{Board, Layout, Outcome};

    fn cfg(solve_worlds: usize, deep_worlds: usize) -> Search {
        Search {
            solve_worlds,
            leaf_worlds: solve_worlds,
            deep_worlds,
            ..Broadside::new().search
        }
    }

    fn mask(cells: &[usize]) -> u128 {
        cells.iter().fold(0, |m, &c| m | 1 << c)
    }

    fn slot(cells: &[usize]) -> u8 {
        SEGMENTS[cells.len()]
            .iter()
            .position(|&m| m == mask(cells))
            .expect("a straight ship") as u8
    }

    /// A legal game in which every cell outside `actual` and `open` was
    /// missed, then `hit_order` fired.
    fn restricted(actual: Layout, open: u128, hit_order: &[usize]) -> Knowledge {
        let allowed = actual.occupied().0 | open;
        let mut board = Board::new(actual);
        let mut k = Knowledge::new();
        for c in (0..CELLS).filter(|&c| allowed >> c & 1 == 0) {
            k.record(c, board.fire(c));
        }
        for &c in hit_order {
            let o = board.fire(c);
            assert_ne!(o, Outcome::Miss);
            k.record(c, o);
        }
        k
    }

    /// Two layouts left; H2 and H3 both certainly hold a ship, but only H3
    /// tells the layouts apart (it sinks the Destroyer in one of them).
    fn two_worlds() -> Knowledge {
        let a = Layout::new([0, 40, 143, 80, 145]).unwrap();
        let b = Layout::new([0, 40, 160, 80, 142]).unwrap();
        restricted(
            a,
            b.occupied().0,
            &[0, 1, 2, 3, 4, 20, 21, 22, 23, 40, 41, 42, 81, 82],
        )
    }

    /// Eight layouts; the only certain cell (H2) is a worse shot than H4.
    fn eight_worlds() -> Knowledge {
        let actual = Layout::new([
            slot(&[0, 1, 2, 3, 4]),
            slot(&[20, 21, 22, 23]),
            slot(&[72, 73, 74]),
            slot(&[40, 41, 42]),
            slot(&[61, 71]),
        ])
        .unwrap();
        restricted(
            actual,
            mask(&[61, 70, 71, 72, 73, 74, 75, 91]),
            &[0, 1, 2, 3, 4, 20, 21, 22, 23, 40, 41, 42, 74],
        )
    }

    /// Nine places left for the Destroyer; greedy continuation misjudges J3.
    fn nine_worlds() -> Knowledge {
        let actual = Layout::new([
            slot(&[0, 1, 2, 3, 4]),
            slot(&[20, 21, 22, 23]),
            slot(&[40, 41, 42]),
            slot(&[60, 61, 62]),
            slot(&[76, 77]),
        ])
        .unwrap();
        restricted(
            actual,
            mask(&[76, 77, 79, 82, 85, 88, 89, 91, 92, 93, 95, 97, 98]),
            &[0, 1, 2, 3, 4, 20, 21, 22, 23, 40, 41, 42, 60, 61, 62],
        )
    }

    /// Optimal total shots by plain search over every shot: no pruning, no
    /// forced moves, no memo.
    fn brute(worlds: &[World], hits: u128) -> f64 {
        let s = Summary::of(worlds.iter(), hits);
        if let Some(v) = s.settled() {
            return v;
        }
        Bits(s.or)
            .map(|x| {
                let mut groups: [Vec<World>; OUTCOMES] = Default::default();
                for w in worlds {
                    groups[outcome(w, x, hits)].push(*w);
                }
                s.weight
                    + (0..OUTCOMES)
                        .filter(|&o| !groups[o].is_empty())
                        .map(|o| brute(&groups[o], advance(o, x, hits)))
                        .sum::<f64>()
            })
            .fold(f64::INFINITY, f64::min)
    }

    fn solved(k: &Knowledge) -> (f64, usize) {
        let w = worlds(k, &Posterior::compute(k), 64, None).unwrap();
        let idx: Vec<u32> = (0..w.len() as u32).collect();
        Tree::new(&w, cfg(64, 0)).solve(&idx, k.hits.0, 64).unwrap()
    }

    #[test]
    fn a_certain_hit_is_not_always_the_right_shot() {
        // Firing H3 separates the two layouts: 3 shots either way (6 in
        // total). H2 leaves a coin flip for the last Cruiser cell: 7.
        let k = two_worlds();
        assert_eq!(Posterior::compute(&k).layouts, 2);
        assert_eq!(solved(&k), (6.0, 72));
        assert_eq!(Broadside::new().choose(&k, &mut Rng::new(7)), 72);

        // H2, the only certain cell, takes 39 shots over the eight layouts
        // even with perfect play afterwards; H4 takes 37.
        let k = eight_worlds();
        assert_eq!(Posterior::compute(&k).layouts, 8);
        assert_eq!(solved(&k), (37.0, 73));
        let w = worlds(&k, &Posterior::compute(&k), 64, None).unwrap();
        assert_eq!(brute(&w, k.hits.0), 37.0);
        assert_eq!(Broadside::new().choose(&k, &mut Rng::new(7)), 73);
    }

    #[test]
    fn solver_beats_greedy_continuation() {
        let k = nine_worlds();
        assert_eq!(Posterior::compute(&k).layouts, 9);
        assert_eq!(solved(&k), (36.0, 92));
        // Rollouts alone settle for I10, at 37 shots over the nine layouts.
        let w = worlds(&k, &Posterior::compute(&k), 64, None).unwrap();
        assert_eq!(Tree::new(&w, cfg(0, 0)).choose(k.hits.0, 1), 89);
        assert_eq!(Tree::new(&w, cfg(0, 0)).choose(k.hits.0, 4), 89);
        assert_eq!(Broadside::new().choose(&k, &mut Rng::new(7)), 92);
    }

    #[test]
    fn prior_weighs_only_unshot_cells() {
        // One Cruiser cell left, J2 or I1; the prior makes I1 twice as
        // likely. The weight on H2 must not matter: it is hit in both.
        let mut k = two_worlds();
        k.record(71, Outcome::Hit);
        k.record(72, Outcome::Sunk(Ship::Destroyer));
        let mut theta = [0.0; CELLS];
        theta[80] = 2f64.ln();
        for h2 in [4f64.ln(), 0.0] {
            theta[71] = h2;
            let tilt = Tilt::from_theta(theta);
            let w = worlds(&k, &Posterior::compute(&k), 64, Some(&tilt)).unwrap();
            let (j2, i1) = (w[0].weight, w[1].weight);
            assert!(w[0].all >> 91 & 1 == 1 && (i1 / j2 - 2.0).abs() < 1e-12);
            let mut engine = Broadside::new();
            engine.prior = Some(std::sync::Arc::new(tilt));
            assert_eq!(engine.choose(&k, &mut Rng::new(1)), 80);
        }
    }

    /// Late positions of games played by a shooter that likes neighbours of
    /// hits, with at most `limit` worlds.
    fn late_states(seed: u64, n: usize, limit: usize) -> Vec<Knowledge> {
        let mut rng = Rng::new(seed);
        let mut out = Vec::new();
        while out.len() < n {
            let mut board = Board::new(Layout::random(&mut rng));
            let mut k = Knowledge::new();
            let shots = 50 + rng.below(35);
            for _ in 0..shots {
                if k.won() {
                    break;
                }
                let near = crate::bitboard::neighbours4(k.hits.0) & k.unknown().0;
                let c = if near != 0 && rng.below(4) != 0 {
                    rng.pick_bit(near)
                } else {
                    rng.pick_bit(k.unknown().0)
                };
                k.record(c, board.fire(c));
            }
            if !k.won() && worlds(&k, &Posterior::compute(&k), limit, None).is_some() {
                out.push(k);
            }
        }
        out
    }

    #[test]
    fn solver_matches_plain_search() {
        let mut checked = 0;
        for k in late_states(11, 200, 6) {
            let w = worlds(&k, &Posterior::compute(&k), 6, None).unwrap();
            if w.len() < 3 {
                continue;
            }
            assert_eq!(solved(&k).0, brute(&w, k.hits.0), "{k:?}");
            checked += 1;
        }
        assert!(checked > 30, "{checked} positions");
    }

    #[test]
    fn tree_values_match_rollouts_on_real_posteriors() {
        for k in late_states(12, 12, 60) {
            let w = worlds(&k, &Posterior::compute(&k), 60, None).unwrap();
            let lists = Posterior::compute(&k).enumerate_afloat(&k, 60).unwrap();
            let n = w.len();
            let idx: Vec<u32> = (0..n as u32).collect();
            let (mut y, mut z) = (vec![0u32; n], vec![0u32; n]);
            let mut tree = Tree::new(&w, cfg(0, 0));
            for x in Bits(k.unknown().0).take(12) {
                let mut seen = Default::default();
                let by_rollout: f64 = lists
                    .iter()
                    .map(|(t, n)| *n as f64 * rollout(&k, x, t, None, &mut seen) as f64)
                    .sum();
                let cuts = tree.split(x, k.hits.0, &idx, &mut y);
                let mut by_tree: f64 = w.iter().map(|w| w.weight).sum();
                for o in 0..OUTCOMES {
                    let r = cuts[o]..cuts[o + 1];
                    if !r.is_empty() {
                        let h = advance(o, x, k.hits.0);
                        by_tree += tree.value(&mut y[r.clone()], &mut z[r], &mut [], h, false);
                    }
                }
                assert_eq!(by_tree, by_rollout, "cell {x} in {k:?}");
            }
        }
    }

    #[test]
    fn stronger_continuations_never_cost_shots() {
        for k in late_states(13, 40, 300) {
            let w = worlds(&k, &Posterior::compute(&k), 300, None).unwrap();
            let n = w.len();
            let value = |solve, deep| {
                let mut x: Vec<u32> = (0..n as u32).collect();
                let (mut y, mut z) = (vec![0u32; n], vec![0u32; n]);
                Tree::new(&w, cfg(solve, deep)).value(&mut x, &mut y, &mut z, k.hits.0, true)
            };
            let greedy = value(0, 0);
            assert!(value(0, 300) <= greedy, "{k:?}");
            assert!(value(16, 0) <= greedy, "{k:?}");
        }
    }
}
