//! Terminal play: human vs engine, and advisor mode against outside opponents.

use broadside_core::ai::{self, Shooter};
use broadside_core::bitboard::{SIZE, cell_name, parse_cell};
use broadside_core::fleet::{FLEET, LENGTHS, SEGMENTS};
use broadside_core::{Board, Knowledge, Layout, Outcome, Posterior, Rng, Ship};
use std::io::{self, BufRead, Write};

fn prompt(msg: &str) -> Option<String> {
    print!("{msg}");
    io::stdout().flush().ok()?;
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_string()),
    }
}

fn ship_letter(s: Ship) -> char {
    match s {
        Ship::Carrier => 'C',
        Ship::Battleship => 'B',
        Ship::Cruiser => 'R',
        Ship::Submarine => 'S',
        Ship::Destroyer => 'D',
    }
}

/// What the shooter sees of a board: `.` unknown, `o` miss, `X` hit, `#` sunk.
fn view_cell(k: &Knowledge, sunk_cells: u128, c: usize) -> char {
    if k.misses.has(c) {
        'o'
    } else if sunk_cells >> c & 1 == 1 {
        '#'
    } else if k.hits.has(c) {
        'X'
    } else {
        '.'
    }
}

/// Cells certainly belonging to sunk ships.
fn sunk_cells(k: &Knowledge) -> u128 {
    if k.sunk.iter().all(Option::is_none) {
        return 0;
    }
    let p = Posterior::compute(k);
    (0..FLEET.len())
        .filter(|&s| k.sunk[s].is_some())
        .flat_map(|s| {
            let w = &p.ships[s];
            (0..w.len())
                .filter(move |&slot| w[slot] == p.layouts)
                .map(move |slot| SEGMENTS[LENGTHS[s]][slot])
        })
        .fold(0, |a, m| a | m)
}

/// Prints two 10x10 grids side by side.
fn print_boards(titles: [&str; 2], cell: [&dyn Fn(usize) -> String; 2]) {
    println!();
    println!("   {:<31}   {}", titles[0], titles[1]);
    let header: String = (1..=SIZE).map(|c| format!("{c:>3}")).collect();
    println!("  {header}     {header}");
    for r in 0..SIZE {
        let row = |f: &dyn Fn(usize) -> String| {
            (0..SIZE)
                .map(|c| format!("{:>3}", f(r * SIZE + c)))
                .collect::<String>()
        };
        let l = (b'A' + r as u8) as char;
        println!("{l} {}   {l} {}", row(cell[0]), row(cell[1]));
    }
    println!();
}

/// Heat glyph for a probability.
fn heat(p: f64) -> String {
    match (p * 100.0).round() as u32 {
        0 => " ·".into(),
        100.. => "██".into(),
        x => format!("{x:>2}"),
    }
}

fn ask_layout(rng: &mut Rng) -> Option<Layout> {
    loop {
        let a = prompt("Place your fleet: [r]andom or [m]anual? ")?;
        match a.to_ascii_lowercase().as_str() {
            "" | "r" | "random" => {
                let l = Layout::random(rng);
                let own = |c: usize| {
                    l.ship_at(c)
                        .map_or("·".into(), |s| ship_letter(s).to_string())
                };
                print_boards(["Your fleet", ""], [&own, &|_| String::new()]);
                match prompt("Keep this layout? [Y/n] ")?
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "n" | "no" => continue,
                    _ => return Some(l),
                }
            }
            "m" | "manual" => {
                let mut slots = [0u8; 5];
                let mut used = 0u128;
                let mut i = 0;
                while i < FLEET.len() {
                    let s = FLEET[i];
                    let a = prompt(&format!(
                        "{} ({} cells): top-left cell and h/v, e.g. `B3 h`: ",
                        s.name(),
                        s.size()
                    ))?;
                    let mut parts = a.split_whitespace();
                    let (Some(c), Some(dir)) = (parts.next().and_then(parse_cell), parts.next())
                    else {
                        println!("  could not read that");
                        continue;
                    };
                    let vertical = dir.eq_ignore_ascii_case("v");
                    let slot = 2 * c + vertical as usize;
                    let m = SEGMENTS[s.size()][slot];
                    if m == 0 || m & used != 0 {
                        println!("  does not fit there");
                        continue;
                    }
                    used |= m;
                    slots[i] = slot as u8;
                    i += 1;
                }
                return Layout::new(slots);
            }
            _ => println!("  r or m"),
        }
    }
}

fn outcome_text(o: Outcome) -> String {
    match o {
        Outcome::Miss => "miss".into(),
        Outcome::Hit => "HIT".into(),
        Outcome::Sunk(s) => format!("HIT - {} sunk!", s.name()),
    }
}

pub fn play(
    seed: u64,
    shooter_name: &str,
    placer_name: &str,
    show_heat: bool,
) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let mut engine: Box<dyn Shooter> = ai::shooter(shooter_name).ok_or("unknown shooter")?;
    engine.set_threads(std::thread::available_parallelism().map_or(1, |n| n.get()));
    let engine_layout = ai::placer(placer_name)
        .ok_or("unknown placer")?
        .place(&mut rng);
    let Some(mine) = ask_layout(&mut rng) else {
        return Ok(());
    };

    let mut enemy_board = Board::new(engine_layout);
    let mut my_board = Board::new(mine);
    let (mut you, mut it) = (Knowledge::new(), Knowledge::new());
    let mut you_first = rng.below(2) == 0;
    println!(
        "{} fire first. Enter cells like `E5`; `q` quits.",
        if you_first { "You" } else { "The engine will" }
    );

    loop {
        if you_first {
            let sc = sunk_cells(&you);
            let target = |c: usize| view_cell(&you, sc, c).to_string();
            let p_it = show_heat.then(|| Posterior::compute(&it));
            let own = |c: usize| {
                let shot = it.shots().has(c);
                match mine.ship_at(c) {
                    Some(_) if shot => "X".into(),
                    Some(s) => ship_letter(s).to_string(),
                    None if shot => "o".into(),
                    None => p_it.as_ref().map_or("·".into(), |p| heat(p.prob(c))),
                }
            };
            print_boards(["Enemy waters", "Your fleet"], [&target, &own]);
            let cell = loop {
                let Some(a) = prompt("Your shot: ") else {
                    return Ok(());
                };
                if a.eq_ignore_ascii_case("q") {
                    return Ok(());
                }
                match parse_cell(&a) {
                    Some(c) if you.shots().has(c) => println!("  already fired there"),
                    Some(c) => break c,
                    None => println!("  cells are A1..J10"),
                }
            };
            let o = enemy_board.fire(cell);
            you.record(cell, o);
            println!("  {} -> {}", cell_name(cell), outcome_text(o));
            if you.won() {
                println!(
                    "\nYou sank the whole fleet in {} shots. You win!",
                    you.shots_fired()
                );
                return Ok(());
            }
        }
        you_first = true;
        let cell = engine.choose(&it, &mut rng);
        let o = my_board.fire(cell);
        it.record(cell, o);
        println!(
            "  Engine fires at {} -> {}",
            cell_name(cell),
            outcome_text(o)
        );
        if it.won() {
            let all = engine_layout.occupied().0;
            let reveal = |c: usize| {
                if all >> c & 1 == 1 {
                    engine_layout
                        .ship_at(c)
                        .map_or('?', ship_letter)
                        .to_string()
                } else {
                    view_cell(&you, 0, c).to_string()
                }
            };
            print_boards(["Engine fleet", ""], [&reveal, &|_| String::new()]);
            println!(
                "The engine sank your fleet in {} shots. You lose.",
                it.shots_fired()
            );
            return Ok(());
        }
    }
}

type History = Vec<(usize, Outcome)>;

/// Advisor: suggests shots against any opponent; you report what happened.
pub fn assist(seed: u64, shooter_name: &str) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let mut engine = ai::shooter(shooter_name).ok_or("unknown shooter")?;
    let mut history: History = Vec::new();
    // Suggestion for the current history, so a typo doesn't re-roll ties.
    let mut last: Option<(History, usize)> = None;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    engine.set_threads(threads);
    println!("Report each shot's result: m (miss), h (hit), or the sunk ship: c b r s d");
    println!(
        "(carrier, battleship, cruiser, submarine, destroyer). Fire elsewhere with `E5 m`; `u` undoes; `q` quits."
    );
    loop {
        let mut k = Knowledge::new();
        for &(c, o) in &history {
            k.record(c, o);
        }
        if k.won() {
            println!("All ships sunk in {} shots.", k.shots_fired());
            return Ok(());
        }
        // Reports are checked as they come in, so some layout always fits.
        let p = Posterior::compute_with(&k, threads);
        let suggestion = match &last {
            Some((h, s)) if *h == history => *s,
            _ => {
                let s = engine.choose(&k, &mut rng);
                last = Some((history.clone(), s));
                s
            }
        };
        let sc = sunk_cells(&k);
        let view = |c: usize| view_cell(&k, sc, c).to_string();
        let hm = |c: usize| {
            if c == suggestion {
                "<>".into()
            } else if k.shots().has(c) {
                view_cell(&k, sc, c).to_string()
            } else {
                heat(p.prob(c))
            }
        };
        print_boards(
            ["Board", "Ship probability % (<> = suggested)"],
            [&view, &hm],
        );
        println!("{} consistent layouts.", p.layouts);
        let label = cell_name(suggestion);
        let Some(a) = prompt(&format!("Fire at {label}. Result: ")) else {
            return Ok(());
        };
        let mut parts: Vec<&str> = a.split_whitespace().collect();
        if parts.first().is_some_and(|w| w.eq_ignore_ascii_case("q")) {
            return Ok(());
        }
        if parts.first().is_some_and(|w| w.eq_ignore_ascii_case("u")) {
            history.pop();
            continue;
        }
        let cell = match parts.len() {
            2 => parse_cell(parts.remove(0)),
            _ => Some(suggestion),
        };
        let Some(cell) = cell.filter(|&c| !k.shots().has(c)) else {
            println!("  need an unshot cell");
            continue;
        };
        let o = match parts.first().map(|w| w.to_ascii_lowercase()).as_deref() {
            Some("m" | "miss") => Outcome::Miss,
            Some("h" | "hit") => Outcome::Hit,
            Some("c") => Outcome::Sunk(Ship::Carrier),
            Some("b") => Outcome::Sunk(Ship::Battleship),
            Some("r") => Outcome::Sunk(Ship::Cruiser),
            Some("s") => Outcome::Sunk(Ship::Submarine),
            Some("d") => Outcome::Sunk(Ship::Destroyer),
            _ => {
                println!("  result must be m, h, c, b, r, s or d");
                continue;
            }
        };
        if let Outcome::Sunk(s) = o
            && k.is_sunk(s)
        {
            println!("  the {} is already sunk", s.name());
            continue;
        }
        // A report no fleet layout fits is a typo: refuse it rather than
        // advise (or declare victory) on an impossible board.
        let mut after = k;
        after.record(cell, o);
        if Posterior::compute_with(&after, threads).layouts == 0 {
            println!(
                "  that cannot be: no fleet layout fits it with the earlier reports. Re-enter it, or undo (`u`) an earlier one."
            );
            continue;
        }
        history.push((cell, o));
    }
}
