//! Shooting and placement strategies.

pub mod baseline;
pub mod engine;
pub mod habits;
pub mod placement;
pub mod search;
mod tilts;

use crate::game::{Knowledge, Layout};
use crate::rng::Rng;

/// Picks the next cell to fire at.
pub trait Shooter: Send {
    fn name(&self) -> String;
    /// Must return an unshot cell. Called only while ships remain afloat.
    fn choose(&mut self, k: &Knowledge, rng: &mut Rng) -> usize;
    /// The opponent's layout, revealed after a game.
    fn learn(&mut self, _layout: &Layout) {}
    /// Threads a single decision may use (1 unless told otherwise:
    /// simulations run one game per thread instead).
    fn set_threads(&mut self, _threads: usize) {}
    /// In a game against an opponent, before each shot: our own fleet and
    /// what the opponent knows of it. The opponent fires next after us, so
    /// whoever needs fewer shots from here wins, we on ties.
    fn race(&mut self, _fleet: &Layout, _against: &Knowledge) {}
}

/// Chooses a fleet layout.
pub trait Placer: Send {
    fn name(&self) -> String;
    fn place(&mut self, rng: &mut Rng) -> Layout;
    /// After a game: our layout and the shots the opponent fired at it, in
    /// order (all of them only if the opponent sank us).
    fn learn(&mut self, _layout: &Layout, _opponent_shots: &[usize]) {}
}

/// Every legal layout equally likely.
pub struct UniformPlacer;

impl Placer for UniformPlacer {
    fn name(&self) -> String {
        "uniform".into()
    }
    fn place(&mut self, rng: &mut Rng) -> Layout {
        Layout::random(rng)
    }
}

/// Uniform over layouts where no two ships touch, not even diagonally
/// (a common house rule, and the default in many apps).
pub struct SpacedPlacer;

impl Placer for SpacedPlacer {
    fn name(&self) -> String {
        "spaced".into()
    }
    fn place(&mut self, rng: &mut Rng) -> Layout {
        loop {
            let l = Layout::random(rng);
            let mut halo = 0u128;
            let ok = crate::fleet::FLEET.iter().all(|&s| {
                let m = l.ship(s).0;
                let ok = m & halo == 0;
                halo |= crate::bitboard::grow8(m);
                ok
            });
            if ok {
                return l;
            }
        }
    }
}

/// Uniform over layouts in which every ship shares an edge with another:
/// the opposite habit, and what someone who knows that an opponent expects
/// spaced fleets would place.
pub struct ClumpedPlacer;

impl Placer for ClumpedPlacer {
    fn name(&self) -> String {
        "clumped".into()
    }
    fn place(&mut self, rng: &mut Rng) -> Layout {
        loop {
            let l = Layout::random(rng);
            let ships = crate::fleet::FLEET.map(|s| l.ship(s).0);
            let lonely = |a: usize| {
                let near = crate::bitboard::neighbours4(ships[a]);
                (0..ships.len()).all(|b| b == a || near & ships[b] == 0)
            };
            if !(0..ships.len()).any(lonely) {
                return l;
            }
        }
    }
}

/// Changes habit every `every` games: spaced fleets, then clumped ones,
/// and so on. Whatever an opponent has just learned is wrong next.
pub struct SwapPlacer {
    pub every: u32,
    placed: u32,
}

impl Placer for SwapPlacer {
    fn name(&self) -> String {
        format!("swap{}", self.every)
    }
    fn place(&mut self, rng: &mut Rng) -> Layout {
        let spaced = (self.placed / self.every).is_multiple_of(2);
        self.placed += 1;
        if spaced {
            SpacedPlacer.place(rng)
        } else {
            ClumpedPlacer.place(rng)
        }
    }
}

/// Places against what the engine has learned: it keeps the engine's own
/// belief from the fleets shown so far and picks the kind of fleet the engine
/// now finds least likely, compared with uniform placement. The worst
/// opponent for the habit learning that this family of habits allows.
pub struct ContraryPlacer {
    /// The engine's belief, kept in step with it.
    engine: habits::Habits,
}

impl Default for ContraryPlacer {
    fn default() -> Self {
        Self {
            engine: habits::Habits::neutral(),
        }
    }
}

impl Placer for ContraryPlacer {
    fn name(&self) -> String {
        "contrary".into()
    }
    fn place(&mut self, rng: &mut Rng) -> Layout {
        let belief = self.engine.belief();
        let neutral = habits::neutral();
        // On ties the more marked habit: clumped, then spaced.
        let kind = [2, 0, 1]
            .into_iter()
            .min_by(|&a, &b| (belief[a] / neutral[a]).total_cmp(&(belief[b] / neutral[b])))
            .unwrap();
        self.engine.observe(kind);
        match kind {
            0 => SpacedPlacer.place(rng),
            2 => ClumpedPlacer.place(rng),
            _ => loop {
                let l = Layout::random(rng);
                if l.contact() == 1 {
                    return l;
                }
            },
        }
    }
}

pub const SHOOTERS: &[&str] = &[
    "broadside",
    "broadside:KEY=N,... (exact, cands, solve, nodes, deep, afloat, samples, race, spread; apart, corners, weight)",
    "greedy",
    "density",
    "NAME@tiltA (expects tiltA placement)",
    "parity",
    "hunt-target",
    "random",
];
pub const PLACERS: &[&str] = &[
    "broadside",
    "equilibrium",
    "uniform",
    "spaced",
    "clumped",
    "swapN (spaced and clumped in turn, N games each)",
    "contrary (the kind of fleet the engine expects least)",
    "tiltA (e.g. tilt1.5)",
    "mix:A=P,A=P,...",
];

/// Tilts worth asking for: finite, and not so strong that fitting breaks down.
fn alpha(s: &str) -> Option<f64> {
    s.parse().ok().filter(|a| (-1.0..=4.0).contains(a))
}

/// The engine by name: `broadside`, `greedy`, or `broadside:KEY=N,...` with
/// search settings for tuning experiments.
fn engine(name: &str) -> Option<engine::Broadside> {
    let mut e = match name {
        "greedy" => return Some(engine::Broadside::greedy()),
        "broadside" => return Some(engine::Broadside::new()),
        _ => engine::Broadside::new(),
    };
    let s = &mut e.search;
    // Believed chance, in percent, that the opponent's ships lie apart or
    // meet at corners only.
    let (mut apart, mut corners) = (None, None);
    for setting in name.strip_prefix("broadside:")?.split(',') {
        let (key, v) = setting.split_once('=')?;
        let v: usize = v.parse().ok()?;
        match key {
            "apart" => apart = Some(v as f64 / 100.0),
            "corners" => corners = Some(v as f64 / 100.0),
            // How many tenths of a revealed fleet that belief counts for.
            "weight" => e.habits.games = v as f64 / 10.0,
            "exact" => s.exact_limit = v,
            "cands" => s.candidates = v,
            "solve" => s.solve_worlds = v,
            "nodes" => s.solve_nodes = v as u64,
            "deep" => s.deep_worlds = v,
            "afloat" => s.max_afloat = v,
            "samples" => s.samples = v,
            "race" => s.race = v,
            "spread" => s.spread = v,
            _ => return None,
        }
    }
    let valid = s.candidates >= 1 && s.solve_worlds <= 64 && (s.max_afloat == 0 || s.samples >= 2);
    // Kinds not named share what is left as under uniform placement.
    let n = habits::neutral();
    let share =
        |left: f64, a: usize, b: usize| [left * n[a] / (n[a] + n[b]), left * n[b] / (n[a] + n[b])];
    e.habits.kinds = match (apart, corners) {
        (None, None) => n,
        (Some(a), Some(c)) => [a, c, 1.0 - a - c],
        (Some(a), None) => {
            let [c, e] = share(1.0 - a, 1, 2);
            [a, c, e]
        }
        (None, Some(c)) => {
            let [a, e] = share(1.0 - c, 0, 2);
            [a, c, e]
        }
    };
    let valid = valid && e.habits.kinds.iter().all(|k| (0.0..=1.0).contains(k));
    valid.then_some(e)
}

pub fn shooter(name: &str) -> Option<Box<dyn Shooter>> {
    // NAME@tiltA: the shooter expects its opponent to place like `tiltA`,
    // and does not learn otherwise.
    if let Some((base, a)) = name.split_once("@tilt") {
        let prior = Some(placement::fitted(alpha(a)?));
        return Some(match base {
            "density" => Box::new(baseline::Density { prior }),
            _ => Box::new(engine::Broadside {
                prior,
                learner: None,
                ..engine(base)?
            }),
        });
    }
    Some(match name {
        "density" => Box::new(baseline::Density::default()),
        "parity" => Box::new(baseline::HuntTarget { parity: true }),
        "hunt-target" => Box::new(baseline::HuntTarget { parity: false }),
        "random" => Box::new(baseline::RandomShooter),
        _ => Box::new(engine(name)?),
    })
}

pub fn placer(name: &str) -> Option<Box<dyn Placer>> {
    Some(match name {
        "uniform" => Box::new(UniformPlacer),
        "spaced" => Box::new(SpacedPlacer),
        "clumped" => Box::new(ClumpedPlacer),
        "contrary" => Box::new(ContraryPlacer::default()),
        n if n.starts_with("swap") => Box::new(SwapPlacer {
            every: n[4..].parse().ok().filter(|&n| n > 0)?,
            placed: 0,
        }),
        "broadside" => Box::new(placement::AdaptivePlacer::new()),
        "equilibrium" => Box::new(placement::MixedPlacer::equilibrium()),
        n if n.starts_with("mix:") => {
            let parts: Vec<(f64, f64)> = n[4..]
                .split(',')
                .map(|kv| {
                    let (a, p) = kv.split_once('=')?;
                    Some((alpha(a)?, p.parse().ok()?))
                })
                .collect::<Option<_>>()?;
            Box::new(placement::MixedPlacer::new(&parts)?)
        }
        n if n.starts_with("tilt") => Box::new(placement::TiltedPlacer::new(alpha(&n[4..])?)),
        _ => return None,
    })
}
