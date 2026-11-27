//! Logs every decision of greedy games as CSV, for measuring what a hit on
//! each ship is worth:
//!
//! `cargo run --release -p broadside-core --example hunt_probe -- GAMES SEED SHOOTER > probe.csv`
//!
//! One row per shot: the position before it (ships afloat, unshot cells,
//! per-ship entropy of where the ship lies), what the shot did, and how
//! many misses the rest of the game took.

use broadside_core::fleet::{LENGTHS, SHIPS};
use broadside_core::{Board, Knowledge, Layout, Outcome, Posterior, Rng, ai};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).map_or(2000, |a| a.parse().unwrap());
    let seed: u64 = args.get(2).map_or(1, |a| a.parse().unwrap());
    let name = args.get(3).map_or("greedy", |s| s.as_str());
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let next = std::sync::atomic::AtomicU64::new(0);
    let out = std::sync::Mutex::new(std::io::BufWriter::new(std::io::stdout()));
    writeln!(
        out.lock().unwrap(),
        "game,n,afloat,open,unknown,lnl,h0,h1,h2,h3,h4,p0,p1,p2,p3,p4,hit,ship,sunk,rest"
    )
    .unwrap();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut shooter = ai::shooter(name).unwrap();
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= games {
                        break;
                    }
                    let layout = Layout::random(&mut Rng::stream(seed, 2 * i));
                    let mut rng = Rng::stream(seed, 2 * i + 1);
                    let mut board = Board::new(layout);
                    let mut k = Knowledge::new();
                    let mut rows: Vec<(String, bool)> = Vec::new();
                    while !k.won() {
                        let p = Posterior::cached(&k, 1);
                        let sunk_len: u32 = (0..SHIPS)
                            .filter(|&s| k.sunk[s].is_some())
                            .map(|s| LENGTHS[s] as u32)
                            .sum();
                        let open = k.hits.count() - sunk_len;
                        let mut h = [0.0; SHIPS];
                        for s in (0..SHIPS).filter(|&s| k.sunk[s].is_none()) {
                            for &w in p.ships[s].iter().filter(|&&w| w > 0) {
                                let q = w as f64 / p.layouts as f64;
                                h[s] -= q * q.ln();
                            }
                        }
                        let cell = shooter.choose(&k, &mut rng);
                        let by_ship = p.ship_cells();
                        let pc: Vec<String> = (0..SHIPS)
                            .map(|s| {
                                if k.sunk[s].is_some() {
                                    "0".into()
                                } else {
                                    format!("{:.4}", by_ship[s][cell] as f64 / p.layouts as f64)
                                }
                            })
                            .collect();
                        let o = board.fire(cell);
                        let ship = layout.ship_at(cell).map_or(-1, |s| s.index() as i32);
                        let row = format!(
                            "{i},{},{},{open},{},{:.3},{},{},{},{},{}",
                            k.shots_fired(),
                            k.afloat_mask(),
                            k.unknown().count(),
                            (p.layouts as f64).ln(),
                            h.map(|x| format!("{x:.3}")).join(","),
                            pc.join(","),
                            (o != Outcome::Miss) as u8,
                            ship,
                            matches!(o, Outcome::Sunk(_)) as u8,
                        );
                        rows.push((row, o == Outcome::Miss));
                        k.record(cell, o);
                    }
                    // Misses after each shot, this one not counted.
                    let mut rest = 0;
                    let mut text = String::new();
                    for (row, miss) in rows.iter().rev() {
                        text = format!("{row},{rest}\n{text}");
                        rest += *miss as u32;
                    }
                    out.lock().unwrap().write_all(text.as_bytes()).unwrap();
                }
            });
        }
    });
}
