//! Records whole games of the engine against random fleets, as JSON, for the
//! replay on the website:
//!
//! `cargo run --release -p broadside-core --example record_games > site/games.json`
//!
//! Each game is a fresh engine (no learning between games) against a uniformly
//! placed fleet. Only games close to the engine's mean (39 to 47 shots) are
//! kept, so the replay shows typical play rather than lucky or unlucky games.

use broadside_core::{FLEET, Layout, Rng, ai, sim};

const GAMES: usize = 8;
const SEED: u64 = 2026;

fn main() {
    let mut rng = Rng::new(SEED);
    let mut games = Vec::new();
    while games.len() < GAMES {
        let layout = Layout::random(&mut rng);
        let mut shooter = ai::shooter("broadside").expect("the engine is a known shooter");
        let shots = sim::play_game(shooter.as_mut(), layout, &mut rng);
        if !(39..=47).contains(&shots.len()) {
            continue;
        }
        let ships: Vec<String> = FLEET
            .iter()
            .map(|&s| format!("[{}]", list(layout.ship(s).iter())))
            .collect();
        games.push(format!(
            "{{\"ships\":[{}],\"shots\":[{}]}}",
            ships.join(","),
            list(shots.into_iter())
        ));
    }
    println!("[\n{}\n]", games.join(",\n"));
}

fn list(cells: impl Iterator<Item = usize>) -> String {
    cells.map(|c| c.to_string()).collect::<Vec<_>>().join(",")
}
