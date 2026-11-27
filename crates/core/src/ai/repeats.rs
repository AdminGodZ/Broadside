//! Whether the opponent puts ships back where they lay before.
//!
//! Many players keep a fleet from game to game, or move a ship or two. The
//! cell habits of `Learner` see that only slowly: they pool all ships and
//! many games. Here every ship's own placements are remembered, recent games
//! most, and a view that expects them again is scored against uniform
//! placement on each fleet before it is counted. The view is trusted by how
//! well it has predicted, so against an opponent who does not repeat it
//! stays out of the way, and one who stops repeating is believed again
//! after a game or two.

use crate::fleet::{SHIPS, SLOTS};
use crate::game::{Knowledge, Layout};
use crate::posterior::Posterior;
use std::sync::Arc;

/// ln of how much likelier than under uniform placement each ship lies at
/// each slot.
pub type SlotWeights = [[f64; SLOTS]; SHIPS];

/// How firmly each view holds on to uniform placement, in games.
const SMOOTH: [f64; VIEWS] = [0.25, 1.0];
const VIEWS: usize = 2;
/// What a game counts for one game later: recent fleets matter most.
const FADE: f64 = 0.6;
/// Bounds on a view's score (ln of its likelihood against uniform
/// placement), so that neither a long habit nor a long absence of one takes
/// more than a game or two to unlearn.
const SCORE: (f64, f64) = (-12.0, 16.0);
/// Below this share of belief the views are left out altogether: one ship
/// that happens to lie where it lay before is no habit.
const NEGLIGIBLE: f64 = 0.01;

#[derive(Clone)]
pub struct Repeats {
    /// How often each ship lay at each slot, faded by age.
    seen: Box<[[f64; SLOTS]; SHIPS]>,
    /// The games behind `seen`, faded alike.
    games: f64,
    scores: [f64; VIEWS],
    /// Belief, before any fleet is seen, that the opponent repeats.
    pub prior: f64,
}

impl Default for Repeats {
    fn default() -> Self {
        Self::new()
    }
}

impl Repeats {
    /// What a new opponent is taken for: almost surely no repeater. A
    /// fleet shown twice outweighs this many times over.
    pub const PRIOR: f64 = 1e-3;

    pub fn new() -> Self {
        Self {
            seen: Box::new([[0.0; SLOTS]; SHIPS]),
            games: 0.0,
            scores: [0.0; VIEWS],
            prior: Self::PRIOR,
        }
    }

    /// The chance of each slot per ship under uniform placement.
    fn uniform() -> Arc<SlotWeights> {
        static U: std::sync::OnceLock<Arc<SlotWeights>> = std::sync::OnceLock::new();
        U.get_or_init(|| {
            let p = Posterior::cached(&Knowledge::new(), 1);
            let mut u = [[0.0; SLOTS]; SHIPS];
            for (row, counts) in u.iter_mut().zip(p.ships.iter()) {
                for (x, &n) in row.iter_mut().zip(counts) {
                    *x = n as f64 / p.layouts as f64;
                }
            }
            Arc::new(u)
        })
        .clone()
    }

    /// How much likelier than uniform view `i` holds `ship` at `slot`.
    fn ratio(&self, i: usize, ship: usize, slot: usize, u: &SlotWeights) -> f64 {
        let k = SMOOTH[i];
        (self.seen[ship][slot] / u[ship][slot] + k) / (self.games + k)
    }

    /// Share of belief in each view, the rest being uniform placement.
    fn belief(&self) -> [f64; VIEWS] {
        let mut w = [0.0; VIEWS];
        for (w, s) in w.iter_mut().zip(self.scores) {
            *w = self.prior / VIEWS as f64 * s.exp();
        }
        let z = 1.0 - self.prior + w.iter().sum::<f64>();
        w.map(|x| x / z)
    }

    /// The belief that the opponent repeats.
    pub fn trust(&self) -> f64 {
        self.belief().iter().sum()
    }

    /// A fleet of the opponent came to light.
    pub fn observe(&mut self, layout: &Layout) {
        let u = Self::uniform();
        if self.games > 0.0 {
            for i in 0..VIEWS {
                let gain: f64 = (0..SHIPS)
                    .map(|s| self.ratio(i, s, layout.slots[s] as usize, &u).ln())
                    .sum();
                self.scores[i] = (self.scores[i] + gain).clamp(SCORE.0, SCORE.1);
            }
        }
        for x in self.seen.iter_mut().flatten() {
            *x *= FADE;
        }
        self.games = self.games * FADE + 1.0;
        for (s, &slot) in layout.slots.iter().enumerate() {
            self.seen[s][slot as usize] += 1.0;
        }
    }

    /// What is expected of the next fleet, as ln weights per ship and slot;
    /// `None` while that is uniform placement for all practical purposes.
    pub fn weights(&self) -> Option<Arc<SlotWeights>> {
        let belief = self.belief();
        if self.games == 0.0 || belief.iter().sum::<f64>() < NEGLIGIBLE {
            return None;
        }
        let u = Self::uniform();
        let rest = 1.0 - belief.iter().sum::<f64>();
        let mut out = [[0.0; SLOTS]; SHIPS];
        for (s, row) in out.iter_mut().enumerate() {
            for (slot, x) in row.iter_mut().enumerate() {
                if u[s][slot] > 0.0 {
                    let w: f64 = (0..VIEWS)
                        .map(|i| belief[i] * self.ratio(i, s, slot, &u))
                        .sum();
                    *x = (rest + w).ln();
                }
            }
        }
        Some(Arc::new(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn a_fleet_shown_twice_is_expected_again() {
        let mut rng = Rng::new(1);
        let fleet = Layout::random(&mut rng);
        let mut r = Repeats::new();
        assert!(r.weights().is_none());
        r.observe(&fleet);
        // Once is no habit yet.
        assert!(r.trust() < 0.01);
        r.observe(&fleet);
        assert!(r.trust() > 0.99, "{}", r.trust());
        let w = r.weights().unwrap();
        for s in 0..SHIPS {
            let at = w[s][fleet.slots[s] as usize];
            assert!(at > 50f64.ln(), "ship {s}: {at}");
            let other = (0..SLOTS).find(|&x| x != fleet.slots[s] as usize && w[s][x] != 0.0);
            assert!(w[s][other.unwrap()] < 0.0);
        }
    }

    #[test]
    fn random_fleets_leave_no_belief() {
        let mut rng = Rng::new(2);
        let mut r = Repeats::new();
        for _ in 0..200 {
            r.observe(&Layout::random(&mut rng));
            assert!(r.weights().is_none(), "trust {}", r.trust());
        }
    }

    #[test]
    fn a_habit_dropped_is_forgotten_within_three_games() {
        let mut rng = Rng::new(3);
        let fleet = Layout::random(&mut rng);
        let mut r = Repeats::new();
        for _ in 0..30 {
            r.observe(&fleet);
        }
        assert!(r.trust() > 0.99);
        for _ in 0..3 {
            r.observe(&Layout::random(&mut rng));
        }
        assert!(r.trust() < 0.01, "{}", r.trust());
        // And picked up again as fast.
        let next = Layout::random(&mut rng);
        for _ in 0..3 {
            r.observe(&next);
        }
        assert!(r.trust() > 0.99, "{}", r.trust());
    }

    #[test]
    fn moving_some_ships_still_counts() {
        let mut rng = Rng::new(4);
        let mut r = Repeats::new();
        let mut fleet = Layout::random(&mut rng);
        r.observe(&fleet);
        for _ in 0..4 {
            // Carrier and Battleship stay, the rest is placed anew.
            loop {
                let other = Layout::random(&mut rng);
                let mut slots = other.slots;
                slots[0] = fleet.slots[0];
                slots[1] = fleet.slots[1];
                if let Some(l) = Layout::new(slots) {
                    fleet = l;
                    break;
                }
            }
            r.observe(&fleet);
        }
        assert!(r.trust() > 0.9, "{}", r.trust());
        let w = r.weights().unwrap();
        assert!(w[0][fleet.slots[0] as usize] > 20f64.ln());
    }
}
