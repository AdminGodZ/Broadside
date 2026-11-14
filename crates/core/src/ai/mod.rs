//! Shooting and placement strategies.

pub mod baseline;
pub mod engine;
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

pub const SHOOTERS: &[&str] = &[
    "broadside",
    "broadside:KEY=N,... (exact, cands, solve, nodes, deep, afloat, samples)",
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
    for setting in name.strip_prefix("broadside:")?.split(',') {
        let (key, v) = setting.split_once('=')?;
        let v: usize = v.parse().ok()?;
        match key {
            "exact" => s.exact_limit = v,
            "cands" => s.candidates = v,
            "solve" => s.solve_worlds = v,
            "nodes" => s.solve_nodes = v as u64,
            "deep" => s.deep_worlds = v,
            "afloat" => s.max_afloat = v,
            "samples" => s.samples = v,
            _ => return None,
        }
    }
    let valid = s.candidates >= 1 && s.solve_worlds <= 64 && (s.max_afloat == 0 || s.samples >= 2);
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
