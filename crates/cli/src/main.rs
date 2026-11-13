mod ui;

use broadside_core::ai;
use broadside_core::sim::{self, Player, Stats};
use broadside_core::{Board, Knowledge, Outcome, Posterior, Rng, fleet};
use std::collections::HashMap;
use std::io::Write as _;
use std::process::ExitCode;
use std::time::{Instant, SystemTime};

const USAGE: &str = "\
Broadside - a Battleship engine

Usage: broadside <command> [options]

Commands:
  play    Play against the engine in the terminal
            --heat  also show where the engine thinks your ships are
  assist  The engine advises your shots against any opponent; you report results
  sim     Shots-to-win statistics of a shooter vs a placer
            --shooter NAME (broadside)  --placer NAME (uniform)
  duel    Head-to-head win rate: duel A B [--placer-a P] [--placer-b P]
  bench   Time the exact posterior on the empty board
  count   Count every legal fleet layout (multithreaded brute force)

Common options: --games N (1000)  --seed S (random)  --threads T (all cores)
";

/// Flags that take no value.
const SWITCHES: &[&str] = &["heat"];

struct Opts {
    pos: Vec<String>,
    flags: HashMap<String, String>,
}

impl Opts {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut pos = Vec::new();
        let mut flags = HashMap::new();
        let mut args = args.peekable();
        while let Some(a) = args.next() {
            if let Some(name) = a.strip_prefix("--")
                && SWITCHES.contains(&name)
            {
                flags.insert(name.to_string(), "true".into());
            } else if let Some(name) = a.strip_prefix("--") {
                let v = args.next().ok_or(format!("--{name} needs a value"))?;
                flags.insert(name.to_string(), v);
            } else {
                pos.push(a);
            }
        }
        Ok(Self { pos, flags })
    }

    fn get<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T, String> {
        match self.flags.get(name) {
            None => Ok(default),
            Some(v) => v
                .parse()
                .map_err(|_| format!("bad value for --{name}: {v}")),
        }
    }

    fn str(&self, name: &str, default: &str) -> String {
        self.flags
            .get(name)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }
}

fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

fn random_seed() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

fn progress(total: u64) -> impl Fn(u64) + Sync {
    use std::io::IsTerminal;
    let tty = std::io::stderr().is_terminal();
    move |done| {
        if tty && (done % (total / 100).max(1) == 0 || done == total) {
            eprint!("\r  {done}/{total}");
            if done == total {
                eprint!("\r\x1b[K");
            }
            let _ = std::io::stderr().flush();
        }
    }
}

fn check_shooter(name: &str) -> Result<(), String> {
    ai::shooter(name).map(|_| ()).ok_or(format!(
        "unknown shooter `{name}` (try: {})",
        ai::SHOOTERS.join(", ")
    ))
}

fn check_placer(name: &str) -> Result<(), String> {
    ai::placer(name).map(|_| ()).ok_or(format!(
        "unknown placer `{name}` (try: {})",
        ai::PLACERS.join(", ")
    ))
}

fn print_stats(label: &str, s: &Stats) {
    println!(
        "{label:<24} mean {:6.3} ± {:.3}  median {:3}  p10 {:3}  p90 {:3}  min {:3}  max {:3}  ({} games)",
        s.mean(),
        s.sem(),
        s.quantile(0.5),
        s.quantile(0.1),
        s.quantile(0.9),
        s.min(),
        s.max(),
        s.games()
    );
}

fn cmd_sim(o: &Opts) -> Result<(), String> {
    let (sh, pl) = (o.str("shooter", "broadside"), o.str("placer", "uniform"));
    check_shooter(&sh)?;
    check_placer(&pl)?;
    let games = o.get("games", 1000u64)?;
    let seed = o.get("seed", random_seed())?;
    let start = Instant::now();
    let stats = sim::benchmark(
        &|| ai::shooter(&sh).unwrap(),
        &|| ai::placer(&pl).unwrap(),
        games,
        seed,
        o.get("threads", threads())?,
        &progress(games),
    );
    print_stats(&format!("{sh} vs {pl}"), &stats);
    println!(
        "seed {seed}, {:.2?} ({:.2?}/game)",
        start.elapsed(),
        start.elapsed() / games.max(1) as u32
    );
    Ok(())
}

fn cmd_duel(o: &Opts) -> Result<(), String> {
    let [a, b] = [1, 2].map(|i| o.pos.get(i).cloned().unwrap_or_default());
    check_shooter(&a)?;
    check_shooter(&b)?;
    let (pa, pb) = (o.str("placer-a", "uniform"), o.str("placer-b", "uniform"));
    check_placer(&pa)?;
    check_placer(&pb)?;
    let games = o.get("games", 1000u64)?;
    let seed = o.get("seed", random_seed())?;
    let r = sim::duel(
        &Player {
            shooter: &|| ai::shooter(&a).unwrap(),
            placer: &|| ai::placer(&pa).unwrap(),
        },
        &Player {
            shooter: &|| ai::shooter(&b).unwrap(),
            placer: &|| ai::placer(&pb).unwrap(),
        },
        games,
        seed,
        o.get("threads", threads())?,
        &progress(games),
    );
    let p = r.a_rate();
    let se = (p * (1.0 - p) / games as f64).sqrt();
    println!(
        "{a} ({pa}) vs {b} ({pb}): {} - {}   A wins {:.2}% ± {:.2}",
        r.a_wins,
        r.b_wins,
        100.0 * p,
        100.0 * se
    );
    print_stats(&format!("  {a} shots"), &r.a_stats);
    print_stats(&format!("  {b} shots"), &r.b_stats);
    println!("seed {seed}");
    Ok(())
}

/// Where the misses happen: hunting (every hit explained by sunk ships) or
/// targeting (some hit belongs to a ship still afloat).
fn cmd_analyze(o: &Opts) -> Result<(), String> {
    let (sh, pl) = (o.str("shooter", "broadside"), o.str("placer", "uniform"));
    check_shooter(&sh)?;
    check_placer(&pl)?;
    let games = o.get("games", 1000u64)?;
    let seed = o.get("seed", random_seed())?;
    let next = std::sync::atomic::AtomicU64::new(0);
    // hunt shots, hunt misses, target shots, target misses, then hunt misses by ships afloat (1..=5)
    let totals = std::sync::Mutex::new([0u64; 10]);
    std::thread::scope(|s| {
        for _ in 0..o.get("threads", threads()).unwrap_or(1) {
            s.spawn(|| {
                let (mut shooter, mut placer) =
                    (ai::shooter(&sh).unwrap(), ai::placer(&pl).unwrap());
                let mut t = [0u64; 10];
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= games {
                        break;
                    }
                    let layout = placer.place(&mut Rng::stream(seed, 2 * i));
                    let mut rng = Rng::stream(seed, 2 * i + 1);
                    let mut board = Board::new(layout);
                    let mut k = Knowledge::new();
                    while !k.won() {
                        let sunk_len: u32 = (0..5)
                            .filter(|&s| k.sunk[s].is_some())
                            .map(|s| fleet::LENGTHS[s] as u32)
                            .sum();
                        let target = k.hits.count() > sunk_len;
                        let cell = shooter.choose(&k, &mut rng);
                        let out = board.fire(cell);
                        k.record(cell, out);
                        let base = if target { 2 } else { 0 };
                        t[base] += 1;
                        t[base + 1] += (out == Outcome::Miss) as u64;
                        if !target && out == Outcome::Miss {
                            t[4 + k.afloat_mask().count_ones() as usize] += 1;
                        }
                    }
                }
                let mut tot = totals.lock().unwrap();
                for (a, b) in tot.iter_mut().zip(t) {
                    *a += b;
                }
            });
        }
    });
    let t = totals.into_inner().unwrap();
    let g = games as f64;
    println!("{sh} vs {pl}, {games} games, seed {seed}");
    println!(
        "  hunt:   {:6.3} shots/game, {:6.3} misses",
        t[0] as f64 / g,
        t[1] as f64 / g
    );
    println!(
        "  target: {:6.3} shots/game, {:6.3} misses",
        t[2] as f64 / g,
        t[3] as f64 / g
    );
    println!("  total:  {:6.3} shots/game", (t[0] + t[2]) as f64 / g);
    let by: Vec<String> = (1..=5)
        .map(|a| format!("{a}: {:.2}", t[4 + a] as f64 / g))
        .collect();
    println!("  hunt misses by ships afloat: {}", by.join("  "));
    Ok(())
}

fn cmd_bench(o: &Opts) -> Result<(), String> {
    let k = Knowledge::new();
    for t in [1, o.get("threads", threads())?] {
        let start = Instant::now();
        let p = Posterior::compute_with(&k, t);
        println!(
            "empty board, {t:2} threads: {} layouts in {:.2?}",
            p.layouts,
            start.elapsed()
        );
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let o = Opts::parse(std::env::args().skip(1))?;
    match o.pos.first().map(String::as_str) {
        Some("play") => ui::play(
            o.get("seed", random_seed())?,
            &o.str("shooter", "broadside"),
            &o.str("placer", "uniform"),
            o.flags.contains_key("heat"),
        ),
        Some("assist") => ui::assist(
            o.get("seed", random_seed())?,
            &o.str("shooter", "broadside"),
        ),
        Some("sim") => cmd_sim(&o),
        Some("duel") => cmd_duel(&o),
        Some("bench") => cmd_bench(&o),
        Some("analyze") => cmd_analyze(&o),
        Some("count") => {
            let start = Instant::now();
            let total = fleet::count_layouts();
            println!("layouts (Cruiser != Submarine): {total}");
            println!("layouts (3-ships interchangeable): {}", total / 2);
            println!("time: {:.2?}", start.elapsed());
            Ok(())
        }
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
