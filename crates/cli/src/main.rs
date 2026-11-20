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
  play     Play against the engine in the terminal
             --heat  also show where the engine thinks your ships are
  assist   The engine advises your shots against any opponent; you report results
  sim      Shots-to-win statistics of a shooter vs a placer
             --shooter NAME (broadside)  --placer NAME (uniform)
             --dump FILE  also write every game (layout, shots) as CSV
  compare  Shooters on the same layouts: compare A [A2 ...] B [--placer P]
             every shooter is measured against the last one
  duel     Head-to-head win rate: duel A B [--placer-a P] [--placer-b P]
             --match N  play matches of N games in which both sides may learn
             --dump FILE  also write both sides' shots-to-win histograms as CSV
  analyze  Where a shooter's misses happen
  matrix   Placement game: placer tilt vs shooter's assumed tilt
             --alphas 0,1,2  --base density|greedy|broadside
  bench    Time the exact posterior and the engine's moves
  count    Count every legal fleet layout (multithreaded brute force)

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

impl Opts {
    fn threads(&self) -> Result<usize, String> {
        match self.get("threads", threads())? {
            0 => Err("--threads must be at least 1".into()),
            t => Ok(t),
        }
    }
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
    let played = sim::games(
        &|| ai::shooter(&sh).unwrap(),
        &|| ai::placer(&pl).unwrap(),
        games,
        o.get("match", 1u64)?,
        seed,
        o.threads()?,
        &progress(games),
    );
    let mut stats = Stats::default();
    for g in &played {
        stats.add(g.shots);
    }
    print_stats(&format!("{sh} vs {pl}"), &stats);
    println!(
        "seed {seed}, {:.2?} ({:.2?}/game)",
        start.elapsed(),
        start.elapsed() / games.max(1) as u32
    );
    if let Some(path) = o.flags.get("dump") {
        let mut csv = String::from("game,shots,carrier,battleship,cruiser,submarine,destroyer\n");
        for (i, g) in played.iter().enumerate() {
            let slots = g.layout.slots.map(|s| s.to_string()).join(",");
            csv += &format!("{i},{},{slots}\n", g.shots);
        }
        std::fs::write(path, csv).map_err(|e| format!("{path}: {e}"))?;
        println!("wrote {path} (ship columns are placement slots: 2 * top-left cell + vertical)");
    }
    Ok(())
}

/// Two shooters against the same layouts, game by game. The difference per
/// game varies far less than either shooter's score, so small gains show up
/// in a fraction of the games an unpaired comparison needs.
fn cmd_compare(o: &Opts) -> Result<(), String> {
    let names: Vec<String> = o.pos[1..].to_vec();
    if names.len() < 2 {
        return Err("compare needs two shooters or more: compare A [A2 ...] B".into());
    }
    for n in &names {
        check_shooter(n)?;
    }
    let pl = o.str("placer", "uniform");
    check_placer(&pl)?;
    let games = o.get("games", 1000u64)?;
    let match_len = o.get("match", 1u64)?;
    let seed = o.get("seed", random_seed())?;
    let threads = o.threads()?;
    let run = |name: &str| {
        let start = Instant::now();
        let played = sim::games(
            &|| ai::shooter(name).unwrap(),
            &|| ai::placer(&pl).unwrap(),
            games,
            match_len,
            seed,
            threads,
            &progress(games),
        );
        let mut stats = Stats::default();
        for g in &played {
            stats.add(g.shots);
        }
        print_stats(&format!("{name} vs {pl}"), &stats);
        println!("  {:.2?}/game", start.elapsed() / games.max(1) as u32);
        played
    };
    // The last shooter is the one the others are measured against.
    let (b, others) = names.split_last().unwrap();
    let gb = run(b);
    for a in others {
        let ga = run(a);
        let d: Vec<f64> = ga
            .iter()
            .zip(&gb)
            .map(|(x, y)| y.shots as f64 - x.shots as f64)
            .collect();
        let n = d.len() as f64;
        let mean = d.iter().sum::<f64>() / n;
        let sd = (d.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0)).sqrt();
        let (se, wins) = (sd / n.sqrt(), |f: fn(f64) -> bool| {
            d.iter().filter(|&&x| f(x)).count()
        });
        println!(
            "{a} saves {mean:+.4} ± {se:.4} shots per game over {b} (95% interval {:+.4} to {:+.4})",
            mean - 1.96 * se,
            mean + 1.96 * se
        );
        println!(
            "  better in {} games, worse in {}, equal in {}; seed {seed}",
            wins(|x| x > 0.0),
            wins(|x| x < 0.0),
            wins(|x| x == 0.0)
        );
    }
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
    let match_len = o.get("match", 1u64)?;
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
        match_len,
        seed,
        o.threads()?,
        &progress(games.div_ceil(match_len.max(1))),
    );
    println!(
        "{a} ({pa}) vs {b} ({pb}): {} - {}   A wins {:.2}% ± {:.2}",
        r.a_wins,
        r.b_wins,
        100.0 * r.a_rate(),
        100.0 * r.a_rate_se()
    );
    if match_len > 1 {
        let late = r
            .a_rate_late()
            .map_or("n/a".into(), |p| format!("{:.2}%", 100.0 * p));
        println!(
            "  {} matches of {match_len} (the error bar is between matches): A wins {late} of the {} games in second halves",
            r.matches.len(),
            r.a_wins_late + r.b_wins_late
        );
    }
    print_stats(&format!("  {a} shots"), &r.a_stats);
    print_stats(&format!("  {b} shots"), &r.b_stats);
    println!("seed {seed}");
    if let Some(path) = o.flags.get("dump") {
        let mut csv = String::from("shots,a_games,b_games\n");
        for (n, (x, y)) in r.a_stats.hist.iter().zip(&r.b_stats.hist).enumerate() {
            csv += &format!("{n},{x},{y}\n");
        }
        std::fs::write(path, csv).map_err(|e| format!("cannot write {path}: {e}"))?;
    }
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
    let threads = o.threads()?;
    let next = std::sync::atomic::AtomicU64::new(0);
    // hunt shots, hunt misses, target shots, target misses, then hunt misses by ships afloat (1..=5)
    let totals = std::sync::Mutex::new([0u64; 10]);
    std::thread::scope(|s| {
        for _ in 0..threads {
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

/// Placement game: mean shots for every (placer tilt, shooter's assumed tilt).
/// Rows are placers; the placer wants the largest row minimum.
fn cmd_matrix(o: &Opts) -> Result<(), String> {
    let alphas: Vec<f64> = o
        .str("alphas", "0,0.5,1,1.5,2,2.5,3")
        .split(',')
        .map(|a| a.parse().map_err(|_| format!("bad alpha {a}")))
        .collect::<Result<_, _>>()?;
    let games = o.get("games", 20000u64)?;
    let seed = o.get("seed", random_seed())?;
    let threads = o.threads()?;
    let base = o.str("base", "density");
    check_shooter(&format!("{base}@tilt1"))?;
    print!("placer \\ shooter");
    for b in &alphas {
        print!("{:>9}", format!("@{b}"));
    }
    println!("{:>9}", "min");
    let mut m = vec![vec![0.0; alphas.len()]; alphas.len()];
    for (i, a) in alphas.iter().enumerate() {
        let placer = format!("tilt{a}");
        print!("{placer:<16}");
        for (j, b) in alphas.iter().enumerate() {
            let sh = format!("{base}@tilt{b}");
            let s = sim::benchmark(
                &|| ai::shooter(&sh).unwrap(),
                &|| ai::placer(&placer).unwrap(),
                games,
                seed,
                threads,
                &|_| {},
            );
            m[i][j] = s.mean();
            print!("{:>9.3}", s.mean());
            let _ = std::io::stdout().flush();
        }
        println!(
            "{:>9.3}",
            m[i].iter().cloned().fold(f64::INFINITY, f64::min)
        );
    }
    println!("{base} shooter, {games} games per cell, seed {seed}");
    let (placer_mix, shooter_mix) = solve_game(&m, 200_000);
    let fmt = |mix: &[f64]| {
        alphas
            .iter()
            .zip(mix)
            .filter(|(_, p)| **p > 0.0005)
            .map(|(a, p)| format!("{a}={p:.4}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    // What each mix guarantees in this matrix, whatever the other side does.
    // The matrix's true value lies between the two.
    let (n, dot) = (alphas.len(), |x: &[f64], y: &[f64]| -> f64 {
        x.iter().zip(y).map(|(a, b)| a * b).sum()
    });
    let lower = (0..n)
        .map(|j| (0..n).map(|i| placer_mix[i] * m[i][j]).sum())
        .fold(f64::INFINITY, f64::min);
    let upper = (0..n)
        .map(|i| dot(&m[i], &shooter_mix))
        .fold(f64::NEG_INFINITY, f64::max);
    println!(
        "equilibrium of this measured matrix: {lower:.3} to {upper:.3} shots (gap {:.4})",
        upper - lower
    );
    println!("  placer mix  (tilt α): mix:{}", fmt(&placer_mix));
    println!("  shooter mix (expects α): {}", fmt(&shooter_mix));
    Ok(())
}

/// Mixed equilibrium of a zero-sum matrix game by fictitious play. The row
/// player maximizes `m[i][j]`. Returns (row mix, column mix); the caller
/// checks how close they are to equilibrium.
fn solve_game(m: &[Vec<f64>], rounds: usize) -> (Vec<f64>, Vec<f64>) {
    let (r, c) = (m.len(), m[0].len());
    let (mut row_counts, mut col_counts) = (vec![0u64; r], vec![0u64; c]);
    let (mut row_payoff, mut col_payoff) = (vec![0.0f64; r], vec![0.0f64; c]);
    for _ in 0..rounds {
        let i = (0..r)
            .max_by(|&a, &b| row_payoff[a].total_cmp(&row_payoff[b]))
            .unwrap();
        let j = (0..c)
            .min_by(|&a, &b| col_payoff[a].total_cmp(&col_payoff[b]))
            .unwrap();
        row_counts[i] += 1;
        col_counts[j] += 1;
        for (k, p) in row_payoff.iter_mut().enumerate() {
            *p += m[k][j];
        }
        for (k, p) in col_payoff.iter_mut().enumerate() {
            *p += m[i][k];
        }
    }
    let n = rounds as f64;
    let rows: Vec<f64> = row_counts.iter().map(|&x| x as f64 / n).collect();
    let cols: Vec<f64> = col_counts.iter().map(|&x| x as f64 / n).collect();
    (rows, cols)
}

/// Fits placement tilts precisely and prints them as Rust source for
/// `crates/core/src/ai/tilts.rs`.
fn cmd_fit_tilts(o: &Opts) -> Result<(), String> {
    use broadside_core::ai::placement::Tilt;
    let alphas: Vec<f64> = o
        .str("alphas", "0.25,0.5,0.75,1,1.25,1.5,1.75,2,2.5,3")
        .split(',')
        .map(|a| a.parse().map_err(|_| format!("bad alpha {a}")))
        .collect::<Result<_, _>>()?;
    let rounds = o.get("rounds", 60usize)?;
    let samples = o.get("samples", 1_000_000usize)?;
    let u = Posterior::compute(&Knowledge::new());
    let fits: Vec<(f64, Tilt)> = std::thread::scope(|s| {
        let hs: Vec<_> = alphas
            .iter()
            .map(|&a| s.spawn(move || (a, Tilt::fit(a, rounds, samples, 0xB5))))
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    println!(
        "//! Precomputed placement tilts (see `broadside fit-tilts`): alpha and theta per cell."
    );
    println!("//! Generated with {rounds} rounds of {samples} samples each.");
    println!();
    println!("pub const BAKED: &[(f64, [f64; 100])] = &[");
    for (a, t) in &fits {
        // Check: occupancy against target u^(1-a), normalized to 17 cells.
        let mut rng = Rng::new(99);
        let n = 400_000;
        let mut occ = [0u64; 100];
        for _ in 0..n {
            for c in t.sample(&mut rng).occupied().iter() {
                occ[c] += 1;
            }
        }
        let raw: Vec<f64> = (0..100).map(|c| u.prob(c).powf(1.0 - a)).collect();
        let scale = 17.0 / raw.iter().sum::<f64>();
        let worst = (0..100)
            .map(|c| (occ[c] as f64 / n as f64 / (raw[c] * scale) - 1.0).abs())
            .fold(0.0, f64::max);
        println!(
            "    // alpha {a}: worst cell occupancy off target by {:.2}%",
            100.0 * worst
        );
        let theta: Vec<String> = t.theta.iter().map(|x| format!("{x:.6}")).collect();
        println!("    ({a:?}, [{}]),", theta.join(", "));
    }
    println!("];");
    Ok(())
}

fn cmd_bench(o: &Opts) -> Result<(), String> {
    let k = Knowledge::new();
    for t in [1, o.threads()?] {
        let start = Instant::now();
        let p = Posterior::compute_with(&k, t);
        println!(
            "empty board, {t:2} threads: {} layouts in {:.2?}",
            p.layouts,
            start.elapsed()
        );
    }
    // Realistic positions: greedy games against uniform layouts, sampled
    // after a given number of shots. Timed single-threaded, uncached.
    let mut rng = Rng::new(o.get("seed", 1u64)?);
    let mut shooter = ai::shooter("greedy").unwrap();
    let mut by_phase: Vec<(u32, Vec<Knowledge>)> =
        [10, 20, 30, 40, 50].map(|n| (n, Vec::new())).into();
    for _ in 0..200 {
        let mut board = Board::new(broadside_core::Layout::random(&mut rng));
        let mut k = Knowledge::new();
        while !k.won() {
            if let Some((_, v)) = by_phase.iter_mut().find(|(n, _)| *n == k.shots_fired()) {
                v.push(k);
            }
            let c = shooter.choose(&k, &mut rng);
            let out = board.fire(c);
            k.record(c, out);
        }
    }
    for (n, states) in &by_phase {
        let reps = if *n <= 10 { 1 } else { 20 };
        let start = Instant::now();
        let mut sum = 0u64;
        for _ in 0..reps {
            for k in states {
                sum = sum.wrapping_add(Posterior::compute(k).layouts);
            }
        }
        let per = start.elapsed() / (reps * states.len().max(1)) as u32;
        // The engine's whole decision at the same positions (search included).
        let mut engine = ai::shooter("broadside").unwrap();
        let (mut slowest, start) = (std::time::Duration::ZERO, Instant::now());
        for k in states {
            let t = Instant::now();
            sum = sum.wrapping_add(engine.choose(k, &mut rng) as u64);
            slowest = slowest.max(t.elapsed());
        }
        let per_move = start.elapsed() / states.len().max(1) as u32;
        println!(
            "after {n:2} shots: {per:>10.2?} per posterior, {per_move:>10.2?} per move (slowest {slowest:.2?}; {} positions, checksum {sum})",
            states.len()
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
            &o.str("placer", "broadside"),
            o.flags.contains_key("heat"),
        ),
        Some("assist") => ui::assist(
            o.get("seed", random_seed())?,
            &o.str("shooter", "broadside"),
        ),
        Some("sim") => cmd_sim(&o),
        Some("compare") => cmd_compare(&o),
        Some("duel") => cmd_duel(&o),
        Some("bench") => cmd_bench(&o),
        Some("analyze") => cmd_analyze(&o),
        Some("matrix") => cmd_matrix(&o),
        Some("fit-tilts") => cmd_fit_tilts(&o),
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
