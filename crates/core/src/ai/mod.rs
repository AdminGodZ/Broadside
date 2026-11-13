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

pub const SHOOTERS: &[&str] = &[
    "broadside",
    "greedy",
    "density",
    "parity",
    "hunt-target",
    "random",
];
pub const PLACERS: &[&str] = &[
    "broadside",
    "uniform",
    "spaced",
    "tiltA (e.g. tilt1.5)",
    "mix:A=P,A=P,...",
];

pub fn shooter(name: &str) -> Option<Box<dyn Shooter>> {
    // NAME@tiltA: the shooter expects its opponent to place like `tiltA`.
    if let Some((base, alpha)) = name.split_once("@tilt") {
        let prior = Some(placement::fitted(alpha.parse().ok()?));
        return Some(match base {
            "density" => Box::new(baseline::Density { prior }),
            "greedy" => Box::new(engine::Broadside {
                prior,
                ..engine::Broadside::greedy()
            }),
            "broadside" => Box::new(engine::Broadside {
                prior,
                ..engine::Broadside::new()
            }),
            _ => return None,
        });
    }
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
        "broadside" => Box::new(placement::MixedPlacer::equilibrium()),
        n if n.starts_with("mix:") => {
            let parts: Vec<(f64, f64)> = n[4..]
                .split(',')
                .map(|kv| {
                    let (a, p) = kv.split_once('=')?;
                    Some((a.parse().ok()?, p.parse().ok()?))
                })
                .collect::<Option<_>>()?;
            Box::new(placement::MixedPlacer::new(&parts))
        }
        n if n.starts_with("tilt") => Box::new(placement::TiltedPlacer::new(n[4..].parse().ok()?)),
        _ => return None,
    })
}
