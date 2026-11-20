//! How an opponent spaces its ships, as a belief the engine reasons with
//! exactly and learns from the fleets it gets to see.
//!
//! Every layout is of one of three kinds (`Layout::contact`): all ships
//! apart, some meeting at a corner, some sharing an edge. A belief gives
//! each kind a chance; within a kind every layout is equally likely. The
//! rules alone (uniform placement) are the belief `neutral()`. A player who
//! never lets ships touch puts everything on the first kind, one who likes
//! to stack them on the last, so the same family covers the habit and its
//! opposite.
//!
//! The posterior under such a belief is exact: layouts of each kind are
//! counted separately (`Spacing`), and what a game reveals shifts the kinds'
//! weights by itself. A belief in spaced fleets that the hits contradict
//! simply drops out of the mix.

use super::engine::{Prior, cell_scores};
use super::search::{self, World};
use crate::bitboard::CELLS;
use crate::fleet::SHIPS;
use crate::game::Knowledge;
use crate::posterior::{Posterior, Spacing};

/// Kinds of layouts by how close their ships come.
pub const KINDS: usize = 3;

/// Layouts of each kind on an empty board.
pub fn kind_totals() -> [u64; KINDS] {
    let (free, edges, apart) = (
        Posterior::total(Spacing::Free),
        Posterior::total(Spacing::NoEdges),
        Posterior::total(Spacing::Apart),
    );
    [apart, edges - apart, free - edges]
}

/// The chance of each kind under uniform placement.
pub fn neutral() -> [f64; KINDS] {
    let all = Posterior::total(Spacing::Free) as f64;
    kind_totals().map(|n| n as f64 / all)
}

/// A belief about the opponent's spacing that learns from its fleets
/// without becoming a weakness.
///
/// Several views predict the kind of the next fleet: the starting belief,
/// which never moves, and learners that count the fleets seen and trust the
/// starting belief for more or fewer games. Each view is trusted by how well
/// it has predicted the fleets so far, and the belief is their average by
/// trust. An opponent with a habit is found out by the learners. One who
/// places against whatever the learners expect makes them predict worse than
/// the unmoved view, which then takes over: what such an opponent can gain
/// is bounded, however long the match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Habits {
    /// Chance that a fleet is of each kind, before any is seen.
    pub kinds: [f64; KINDS],
    /// How many revealed fleets that belief counts for; the learners use
    /// multiples of it (`PACE`).
    pub games: f64,
    seen: [u32; KINDS],
    /// Share of trust per view: the unmoved one, then the learners.
    trust: [f64; VIEWS],
}

const VIEWS: usize = 3;
/// How firmly each learner holds the starting belief, in units of `games`.
const PACE: [f64; VIEWS - 1] = [0.5, 2.0];
/// Trust before any fleet is seen.
const TRUST: [f64; VIEWS] = [0.5, 0.25, 0.25];
/// Share of trust handed back to the starting split after every fleet, so
/// that a view which was wrong for long can still recover when the opponent
/// changes its ways.
const RECOVERY: f64 = 0.05;

impl Habits {
    /// Fleets seen before an opponent's habits are trusted over the rules.
    pub const GAMES: f64 = 6.0;

    pub fn neutral() -> Self {
        Self::starting(neutral(), Self::GAMES)
    }

    pub fn starting(kinds: [f64; KINDS], games: f64) -> Self {
        Self {
            kinds,
            games,
            seen: [0; KINDS],
            trust: TRUST,
        }
    }

    /// What view `i` expects of the next fleet.
    fn view(&self, i: usize) -> [f64; KINDS] {
        let n: u32 = self.seen.iter().sum();
        if i == 0 || n == 0 {
            return self.kinds;
        }
        let games = self.games * PACE[i - 1];
        let mut out = [0.0; KINDS];
        for (o, (k, s)) in out.iter_mut().zip(self.kinds.iter().zip(self.seen)) {
            *o = (games * k + s as f64) / (games + n as f64);
        }
        out
    }

    /// The chance of each kind for the opponent's next fleet.
    pub fn belief(&self) -> [f64; KINDS] {
        if self.seen == [0; KINDS] {
            return self.kinds;
        }
        let mut out = [0.0; KINDS];
        for (i, t) in self.trust.iter().enumerate() {
            for (o, v) in out.iter_mut().zip(self.view(i)) {
                *o += t * v;
            }
        }
        out
    }

    /// A fleet of the opponent came to light.
    pub fn observe(&mut self, kind: usize) {
        let mut trust = [0.0; VIEWS];
        for (i, t) in trust.iter_mut().enumerate() {
            // A view sure of the opposite is not written off for good.
            *t = self.trust[i] * self.view(i)[kind].max(1e-6);
        }
        let z: f64 = trust.iter().sum();
        for (i, t) in trust.iter().enumerate() {
            self.trust[i] = (1.0 - RECOVERY) * t / z + RECOVERY * TRUST[i];
        }
        self.seen[kind] += 1;
    }
}

/// The posterior of one position under a belief about spacing.
pub struct Mix {
    /// Layouts consistent with the knowledge, counted under each spacing:
    /// free, no shared edges, apart. Each contains the next.
    nested: [Posterior; KINDS],
    /// How much likelier than under uniform placement a layout of each kind
    /// is.
    density: [f64; KINDS],
    /// The layouts the belief allows at all.
    pub support: Posterior,
}

const GAPS: [Spacing; KINDS] = [Spacing::Free, Spacing::NoEdges, Spacing::Apart];

impl Mix {
    /// `free` must be the posterior of `k`. `None` if the belief allows no
    /// layout that fits what is known: it was wrong, and only the rules are
    /// left to go by.
    pub fn new(
        k: &Knowledge,
        free: &Posterior,
        kinds: &[f64; KINDS],
        threads: usize,
    ) -> Option<Self> {
        let totals = kind_totals();
        let all = Posterior::total(Spacing::Free) as f64;
        let mut density = [0.0; KINDS];
        for (d, (kind, total)) in density.iter_mut().zip(kinds.iter().zip(totals)) {
            *d = kind.max(0.0) * all / total as f64;
        }
        let nested = [
            free.clone(),
            Posterior::cached_spaced(k, Spacing::NoEdges, threads),
            Posterior::cached_spaced(k, Spacing::Apart, threads),
        ];
        let mut support = nested[2].minus(&nested[2]);
        for (kind, _) in density.iter().enumerate().filter(|d| *d.1 > 0.0) {
            support = support.plus(&Self::kind(&nested, kind));
        }
        (support.layouts > 0).then_some(Self {
            nested,
            density,
            support,
        })
    }

    /// The layouts of one kind: apart, corners only, shared edges.
    fn kind(nested: &[Posterior; KINDS], kind: usize) -> Posterior {
        match kind {
            0 => nested[2].clone(),
            1 => nested[1].minus(&nested[2]),
            _ => nested[0].minus(&nested[1]),
        }
    }

    /// Weight of all the layouts the belief allows, in layouts under
    /// uniform placement.
    pub fn weight(&self) -> f64 {
        (0..KINDS)
            .map(|kind| self.density[kind] * Self::kind(&self.nested, kind).layouts as f64)
            .sum()
    }

    /// The chance of each kind given what is known.
    pub fn kinds(&self) -> [f64; KINDS] {
        let mut out = [0.0; KINDS];
        for (kind, o) in out.iter_mut().enumerate() {
            *o = self.density[kind] * Self::kind(&self.nested, kind).layouts as f64;
        }
        let z: f64 = out.iter().sum();
        out.map(|x| x / z)
    }

    /// How much each unshot cell is worth shooting (see `cell_scores`).
    pub fn scores(&self, prior: Prior, k: &Knowledge) -> [f64; CELLS] {
        let mut out = [0.0; CELLS];
        for kind in 0..KINDS {
            let p = Self::kind(&self.nested, kind);
            if self.density[kind] <= 0.0 || p.layouts == 0 {
                continue;
            }
            // Without a prior the scores are layout counts; with one, they
            // are chances within the kind.
            let scale = if prior.is_none() {
                self.density[kind]
            } else {
                self.density[kind] * p.layouts as f64
            };
            for (o, s) in out.iter_mut().zip(cell_scores(&p, prior, k)) {
                *o += scale * s;
            }
        }
        out
    }

    /// Every world the belief allows, weighted by it, or `None` if there are
    /// more than `limit`.
    pub fn worlds(&self, k: &Knowledge, limit: usize, prior: Prior) -> Option<Vec<World>> {
        // The spacings from the widest the belief allows inward: each
        // one's placements contain the next one's, in the same order.
        let widest = (0..KINDS)
            .find(|&g| self.density[KINDS - 1 - g] > 0.0)
            .expect("a belief with support allows some kind");
        let lists = (widest..KINDS)
            .map(|g| self.nested[g].enumerate_spaced(k, limit, GAPS[g]))
            .collect::<Option<Vec<_>>>()?;
        let mut at = vec![0usize; lists.len()];
        let mut out: Vec<([Option<u8>; SHIPS], f64)> = Vec::with_capacity(lists[0].len());
        for &(slots, _) in &lists[0] {
            // Layouts behind this placement under each spacing, then none.
            let mut count = [0u64; KINDS + 1];
            for (i, list) in lists.iter().enumerate() {
                if let Some(&(s, n)) = list.get(at[i])
                    && s == slots
                {
                    at[i] += 1;
                    count[i] = n;
                }
            }
            // The kinds are what one spacing has over the next.
            let weight: f64 = (0..lists.len())
                .map(|i| self.density[KINDS - 1 - widest - i] * (count[i] - count[i + 1]) as f64)
                .sum();
            if weight > 0.0 {
                out.push((slots, weight));
            }
        }
        debug_assert!(at.iter().zip(&lists).all(|(a, l)| *a == l.len()));
        Some(search::build(k, out, prior))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::engine::Broadside;
    use crate::ai::{ClumpedPlacer, Placer, Shooter, SpacedPlacer};
    use crate::bitboard::Bits;
    use crate::game::{Board, Layout};
    use crate::rng::Rng;
    use crate::sim::play_game;

    /// A position some shots into a game on `layout`.
    fn state(layout: Layout, shots: usize, rng: &mut Rng) -> Knowledge {
        let mut board = Board::new(layout);
        let mut k = Knowledge::new();
        for _ in 0..shots {
            if k.won() {
                break;
            }
            let near = crate::bitboard::neighbours4(k.hits.0) & k.unknown().0;
            let pool = if near != 0 && rng.below(4) != 0 {
                near
            } else {
                k.unknown().0
            };
            let c = rng.pick_bit(pool);
            k.record(c, board.fire(c));
        }
        k
    }

    #[test]
    fn kinds_split_every_layout() {
        assert_eq!(
            kind_totals().iter().sum::<u64>(),
            Posterior::total(Spacing::Free)
        );
        assert!((neutral().iter().sum::<f64>() - 1.0).abs() < 1e-12);
        let mut rng = Rng::new(1);
        let mut seen = [0u32; KINDS];
        for _ in 0..4000 {
            seen[Layout::random(&mut rng).contact()] += 1;
        }
        for (s, n) in seen.iter().zip(neutral()) {
            assert!((*s as f64 / 4000.0 - n).abs() < 0.03, "{seen:?}");
        }
        for _ in 0..50 {
            assert_eq!(SpacedPlacer.place(&mut rng).contact(), 0);
            assert_eq!(ClumpedPlacer.place(&mut rng).contact(), 2);
        }
    }

    #[test]
    fn the_neutral_belief_is_the_plain_posterior() {
        let mut rng = Rng::new(2);
        for _ in 0..20 {
            let k = state(Layout::random(&mut rng), 55, &mut rng);
            if k.won() {
                continue;
            }
            let p = Posterior::compute(&k);
            let mix = Mix::new(&k, &p, &neutral(), 1).unwrap();
            assert_eq!(mix.support.cells, p.cells);
            for (a, b) in mix.scores(Prior::NONE, &k).iter().zip(p.cells) {
                assert!((a - b as f64).abs() <= 1e-9 * b as f64);
            }
            let (Some(a), Some(b)) = (
                mix.worlds(&k, 3000, Prior::NONE),
                search::worlds(&k, &p, 3000, Prior::NONE),
            ) else {
                continue;
            };
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                assert_eq!(x.ships, y.ships);
                assert!((x.weight - y.weight).abs() <= 1e-9 * y.weight);
            }
        }
    }

    #[test]
    fn worlds_carry_the_weight_of_the_kinds() {
        let mut rng = Rng::new(3);
        let beliefs = [
            [0.6, 0.1, 0.3],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.5, 0.0, 0.5],
            [0.0, 0.3, 0.7],
        ];
        let mut checked = 0;
        for i in 0..200 {
            let layout = if i % 2 == 0 {
                SpacedPlacer.place(&mut rng)
            } else {
                Layout::random(&mut rng)
            };
            let k = state(layout, 50 + i % 20, &mut rng);
            if k.won() {
                continue;
            }
            let p = Posterior::compute(&k);
            for kinds in &beliefs {
                let Some(mix) = Mix::new(&k, &p, kinds, 1) else {
                    continue;
                };
                let Some(worlds) = mix.worlds(&k, 3000, Prior::NONE) else {
                    continue;
                };
                // Cell by cell, the worlds add up to the exact counts.
                let scores = mix.scores(Prior::NONE, &k);
                let mut occ = [0.0; CELLS];
                for w in &worlds {
                    for c in Bits(w.all & k.unknown().0) {
                        occ[c] += w.weight;
                    }
                }
                for c in Bits(k.unknown().0) {
                    assert!(
                        (occ[c] - scores[c]).abs() <= 1e-9 * scores[c].max(1.0),
                        "{kinds:?} {k:?}"
                    );
                }
                // Posterior chances of the kinds: none for a kind the
                // belief rules out, and they sum to one.
                let post = mix.kinds();
                assert!((post.iter().sum::<f64>() - 1.0).abs() < 1e-9);
                assert!((0..KINDS).all(|kind| kinds[kind] > 0.0 || post[kind] == 0.0));
                checked += 1;
            }
        }
        assert!(checked > 100, "{checked}");
    }

    #[test]
    fn a_wrong_belief_gives_way_to_the_rules() {
        let mut rng = Rng::new(4);
        for sure in [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]] {
            let mut engine = Broadside::new();
            engine.search.exact_limit = 2000;
            engine.habits.kinds = sure;
            for _ in 0..6 {
                // Fleets of every kind, most of them not the expected one.
                for layout in [
                    SpacedPlacer.place(&mut rng),
                    ClumpedPlacer.place(&mut rng),
                    Layout::random(&mut rng),
                ] {
                    let shots = play_game(&mut engine, layout, &mut rng);
                    assert!((17..=100).contains(&shots.len()));
                }
            }
        }
    }

    #[test]
    fn believing_in_spaced_fleets_pays_against_them() {
        let mut rng = Rng::new(5);
        let (mut plain, mut spaced) = (Broadside::greedy(), Broadside::greedy());
        spaced.habits.kinds = [0.9, 0.05, 0.05];
        let (mut a, mut b) = (0, 0);
        for i in 0..60 {
            let layout = SpacedPlacer.place(&mut rng);
            a += play_game(&mut plain, layout, &mut Rng::new(i)).len();
            b += play_game(&mut spaced, layout, &mut Rng::new(i)).len();
        }
        assert!(b + 60 < a, "{b} against {a} shots");
    }

    /// How much worse than the rules alone the belief predicts a run of
    /// fleets, in nats (the sum of ln(neutral / belief) over their kinds).
    fn regret(kinds: impl Iterator<Item = usize>) -> f64 {
        let mut h = Habits::neutral();
        let mut total = 0.0;
        for kind in kinds {
            total += (neutral()[kind] / h.belief()[kind]).ln();
            h.observe(kind);
        }
        total
    }

    #[test]
    fn placing_against_the_belief_gains_little() {
        // The opponent that always shows the kind the engine expects least.
        let mut mirror = Habits::neutral();
        let contrary = std::iter::repeat_with(move || {
            let b = mirror.belief();
            let kind = [2, 0, 1]
                .into_iter()
                .min_by(|&x, &y| (b[x] / neutral()[x]).total_cmp(&(b[y] / neutral()[y])))
                .unwrap();
            mirror.observe(kind);
            kind
        });
        let lost = regret(contrary.take(200));
        assert!(lost < 5.0, "{lost}");
        // A real habit is worth far more than that.
        let gained = -regret(std::iter::repeat_n(0, 20));
        assert!(gained > 20.0, "{gained}");
        // Changing habit every five fleets still leaves the engine ahead.
        let swapped = -regret((0..40).map(|i| if i / 5 % 2 == 0 { 0 } else { 2 }));
        assert!(swapped > 0.0, "{swapped}");
    }

    #[test]
    fn chances_add_up_to_the_ship_cells_left() {
        let mut rng = Rng::new(8);
        let mut believer = Broadside::new();
        believer.habits.kinds = [0.7, 0.1, 0.2];
        for _ in 0..20 {
            let k = state(SpacedPlacer.place(&mut rng), 40, &mut rng);
            if k.won() {
                continue;
            }
            let left = (crate::fleet::FLEET_CELLS - k.hits.count()) as f64;
            for engine in [&Broadside::new(), &believer] {
                let chances = engine.chances(&k);
                assert!((chances.iter().sum::<f64>() - left).abs() < 1e-6);
                assert!(Bits(k.shots().0).all(|c| chances[c] == 0.0));
            }
            // Next to a sunk ship the believer expects less than the rules.
            let p = Posterior::compute(&k);
            let (plain, spaced) = (Broadside::new().chances(&k), believer.chances(&k));
            for s in (0..SHIPS).filter(|&s| k.sunk[s].is_some()) {
                let Some(slot) = (0..p.ships[s].len()).find(|&x| p.ships[s][x] == p.layouts) else {
                    continue;
                };
                let ship = crate::fleet::SEGMENTS[crate::fleet::LENGTHS[s]][slot];
                for c in Bits(crate::bitboard::neighbours4(ship) & k.unknown().0) {
                    assert!(spaced[c] <= plain[c] + 1e-12, "{k:?}");
                }
            }
        }
    }

    #[test]
    fn habits_are_learned_from_revealed_fleets() {
        let mut rng = Rng::new(6);
        let mut engine = Broadside::new();
        assert_eq!(engine.spacing(), None);
        for _ in 0..3 {
            engine.learn(&SpacedPlacer.place(&mut rng));
        }
        // A few fleets move the belief with care, more of them firmly.
        let spaced = engine.spacing().unwrap();
        assert!(spaced[0] > 0.3 && spaced[0] < 0.6, "{spaced:?}");
        for _ in 0..7 {
            engine.learn(&SpacedPlacer.place(&mut rng));
        }
        let spaced = engine.spacing().unwrap();
        assert!(spaced[0] > 0.6, "{spaced:?}");
        // The opposite habit is learned just the same.
        let mut engine = Broadside::new();
        for _ in 0..30 {
            engine.learn(&ClumpedPlacer.place(&mut rng));
        }
        let clumped = engine.spacing().unwrap();
        assert!(clumped[2] > 0.85 && clumped[0] < 0.12, "{clumped:?}");
        // A fixed prior learns nothing.
        let mut fixed = Broadside {
            learner: None,
            ..Broadside::new()
        };
        fixed.learn(&SpacedPlacer.place(&mut rng));
        assert_eq!(fixed.spacing(), None);
    }
}
