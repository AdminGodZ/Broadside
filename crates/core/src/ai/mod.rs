//! Shooting and placement strategies.

pub mod baseline;
pub mod engine;
pub mod placement;

use crate::game::{Knowledge, Layout};
use crate::rng::Rng;

/// Picks the next cell to fire at.
pub trait Shooter: Send {
    fn name(&self) -> String;
    /// Must return an unshot cell. Called only while ships remain afloat.
    fn choose(&mut self, k: &Knowledge, rng: &mut Rng) -> usize;
    /// The opponent's layout, revealed after a game.
    fn learn(&mut self, _layout: &Layout) {}
}

/// Chooses a fleet layout.
pub trait Placer: Send {
    fn name(&self) -> String;
    fn place(&mut self, rng: &mut Rng) -> Layout;
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

/// Uniformly random cell among the `unknown` cells that maximize `score`.
pub(crate) fn argmax_random(
    unknown: u128,
    rng: &mut Rng,
    mut score: impl FnMut(usize) -> u64,
) -> usize {
    let mut best = 0u64;
    let mut ties = 0u128;
    for c in crate::bitboard::Bits(unknown) {
        let s = score(c);
        if s > best || ties == 0 {
            best = s;
            ties = 1 << c;
        } else if s == best {
            ties |= 1 << c;
        }
    }
    rng.pick_bit(ties)
}

pub const SHOOTERS: &[&str] = &[
    "broadside",
    "greedy",
    "density",
    "parity",
    "hunt-target",
    "random",
];
pub const PLACERS: &[&str] = &["uniform", "spaced", "tiltA (e.g. tilt1, tilt1.5)"];

pub fn shooter(name: &str) -> Option<Box<dyn Shooter>> {
    Some(match name {
        "broadside" => Box::new(engine::Broadside::new()),
        "greedy" => Box::new(engine::Broadside::greedy()),
        // broadside:AFLOAT:CANDIDATES:SAMPLES[:EXACT_LIMIT], for tuning experiments.
        n if n.starts_with("broadside:") => {
            let v: Vec<usize> = n
                .split(':')
                .skip(1)
                .map(|x| x.parse().ok())
                .collect::<Option<_>>()?;
            let mut search = engine::Broadside::new().search;
            match v[..] {
                [a, c, s] => (search.max_afloat, search.candidates, search.samples) = (a, c, s),
                [a, c, s, e] => {
                    (
                        search.max_afloat,
                        search.candidates,
                        search.samples,
                        search.exact_limit,
                    ) = (a, c, s, e)
                }
                _ => return None,
            }
            Box::new(engine::Broadside {
                search,
                ..engine::Broadside::new()
            })
        }
        "density" => Box::new(baseline::Density::default()),
        // density@tiltA: density that expects a tilt-A placer.
        n if n.starts_with("density@tilt") => {
            let alpha: f64 = n["density@tilt".len()..].parse().ok()?;
            Box::new(baseline::Density {
                prior: Some(placement::fitted(alpha)),
            })
        }
        "parity" => Box::new(baseline::HuntTarget { parity: true }),
        "hunt-target" => Box::new(baseline::HuntTarget { parity: false }),
        "random" => Box::new(baseline::RandomShooter),
        _ => return None,
    })
}

pub fn placer(name: &str) -> Option<Box<dyn Placer>> {
    Some(match name {
        "uniform" => Box::new(UniformPlacer),
        "spaced" => Box::new(SpacedPlacer),
        n if n.starts_with("tilt") => Box::new(placement::TiltedPlacer::new(n[4..].parse().ok()?)),
        _ => return None,
    })
}
