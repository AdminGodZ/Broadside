//! Diagnostic: one shooter against one placer through long matches, both
//! learning. Prints shots by stretch of the match, and for the match's
//! second half how concentrated each ship's places are and when the shooter
//! first hit it.
//!
//! cargo run --release -p broadside-core --example match_probe -- SHOOTER PLACER MATCHES LEN [THREADS]

use broadside_core::ai;
use broadside_core::bitboard::Bits;
use broadside_core::fleet::FLEET;
use broadside_core::rng::Rng;
use broadside_core::sim::play_game;

#[derive(Default)]
struct Tally {
    shots: Vec<f64>,
    top_slot: [f64; 5],
    top_cell: [f64; 5],
    distinct: [f64; 5],
    first_hit: [f64; 5],
}

fn one_match(sh: &str, pl: &str, m: u64, len: usize, t: &mut Tally) {
    let (mut s, mut p) = (ai::shooter(sh).unwrap(), ai::placer(pl).unwrap());
    let mut slots = vec![vec![0u32; 200]; 5];
    let mut cells = vec![vec![0u32; 100]; 5];
    t.shots.resize(len, 0.0);
    for g in 0..len {
        let i = m * len as u64 + g as u64;
        let layout = p.place(&mut Rng::stream(7, 2 * i));
        let fired = play_game(s.as_mut(), layout, &mut Rng::stream(7, 2 * i + 1));
        s.learn(&layout, &fired);
        p.learn(&layout, &fired);
        t.shots[g] += fired.len() as f64;
        if g < len / 2 {
            continue;
        }
        for (k, ship) in FLEET.into_iter().enumerate() {
            slots[k][layout.slots[k] as usize] += 1;
            let mask = layout.ship(ship).0;
            for c in Bits(mask) {
                cells[k][c] += 1;
            }
            t.first_hit[k] += fired.iter().position(|&c| mask >> c & 1 == 1).unwrap() as f64;
        }
    }
    let half = (len - len / 2) as f64;
    for k in 0..5 {
        t.top_slot[k] += *slots[k].iter().max().unwrap() as f64 / half;
        t.top_cell[k] += *cells[k].iter().max().unwrap() as f64 / half;
        t.distinct[k] += slots[k].iter().filter(|&&n| n > 0).count() as f64;
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (sh, pl) = (a[0].as_str(), a[1].as_str());
    let matches: u64 = a[2].parse().unwrap();
    let len: usize = a[3].parse().unwrap();
    let threads: u64 = a.get(4).map_or(8, |x| x.parse().unwrap());
    let parts: Vec<Tally> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|w| {
                scope.spawn(move || {
                    let mut t = Tally::default();
                    for m in (w..matches).step_by(threads as usize) {
                        one_match(sh, pl, m, len, &mut t);
                    }
                    t
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut t = Tally {
        shots: vec![0.0; len],
        ..Tally::default()
    };
    for p in parts.iter().filter(|p| !p.shots.is_empty()) {
        for g in 0..len {
            t.shots[g] += p.shots[g];
        }
        for k in 0..5 {
            t.top_slot[k] += p.top_slot[k];
            t.top_cell[k] += p.top_cell[k];
            t.distinct[k] += p.distinct[k];
            t.first_hit[k] += p.first_hit[k];
        }
    }
    let n = matches as f64;
    let step = (len / 5).max(1);
    println!("{sh} against {pl}: {matches} matches of {len}");
    for (b, chunk) in t.shots.chunks(step).enumerate() {
        let mean = chunk.iter().sum::<f64>() / (chunk.len() as f64 * n);
        println!("  games {:>3}..: {mean:.2} shots", b * step);
    }
    let half = n * (len - len / 2) as f64;
    println!("  second half, per ship (5,4,3,3,2):");
    println!("    commonest slot  {:.2?}", t.top_slot.map(|x| x / n));
    println!("    commonest cell  {:.2?}", t.top_cell.map(|x| x / n));
    println!("    distinct slots  {:.1?}", t.distinct.map(|x| x / n));
    println!("    first hit after {:.1?}", t.first_hit.map(|x| x / half));
}
