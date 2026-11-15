//! The engine against the exact optimum when the last ship is not yet hit.

use broadside_core::ai::Shooter;
use broadside_core::ai::engine::Broadside;
use broadside_core::ai::last;
use broadside_core::bitboard::Bitboard;
use broadside_core::fleet::{LENGTHS, SEGMENTS, SHIPS};
use broadside_core::game::{Board, Knowledge, Layout};
use broadside_core::rng::Rng;

fn slot(len: usize, mask: u128) -> Option<u8> {
    SEGMENTS[len]
        .iter()
        .position(|&m| m == mask)
        .map(|i| i as u8)
}

fn cells(list: &[usize]) -> u128 {
    list.iter().fold(0, |m, &c| m | 1 << c)
}

/// Four ships sunk where `sunk` says, everything else shot except `open`,
/// ship `last` somewhere in `open`: the shots the engine needs from there,
/// summed over every place the last ship can be, and how many places.
fn total(
    engine: fn() -> Broadside,
    sunk: [(usize, u128); 4],
    last: usize,
    open: u128,
) -> (u32, u32) {
    let mut history: Vec<usize> = Vec::new();
    for (_, m) in sunk {
        history.extend(Bitboard(m).iter());
    }
    let taken = sunk.iter().fold(0, |a, (_, m)| a | m);
    history.extend((0..100).filter(|&c| (open | taken) >> c & 1 == 0));
    let (mut shots, mut places) = (0, 0);
    for &seg in SEGMENTS[LENGTHS[last]].iter() {
        if seg == 0 || seg & !open != 0 {
            continue;
        }
        let mut slots = [0u8; SHIPS];
        for (s, m) in sunk {
            slots[s] = slot(LENGTHS[s], m).unwrap();
        }
        slots[last] = slot(LENGTHS[last], seg).unwrap();
        let mut board = Board::new(Layout::new(slots).unwrap());
        let mut k = Knowledge::new();
        for &c in &history {
            k.record(c, board.fire(c));
        }
        let (mut e, mut rng) = (engine(), Rng::new(1));
        while !k.won() {
            let c = e.choose(&k, &mut rng);
            k.record(c, board.fire(c));
            shots += 1;
        }
        places += 1;
    }
    (shots, places)
}

fn without() -> Broadside {
    let mut e = Broadside::new();
    e.search.last_nodes = 0;
    e
}

/// The search over listed placements alone, given a great deal of time.
fn patient() -> Broadside {
    let mut e = without();
    e.search.solve_worlds = 64;
    e.search.solve_nodes = 3_000_000;
    e
}

/// From the research report of 2026-11-15: 26 places for the Destroyer.
#[test]
fn a_destroyer_in_twenty_cells() {
    let open = cells(&[
        50, 51, 52, 53, 54, 55, 63, 64, 65, 70, 71, 72, 73, 74, 75, 80, 81, 82, 84, 85,
    ]);
    let sunk = [
        (0, cells(&[0, 1, 2, 3, 4])),
        (1, cells(&[20, 21, 22, 23])),
        (2, cells(&[40, 41, 42])),
        (3, cells(&[90, 91, 92])),
    ];
    let order = last::best(Bitboard(open), 2, 1_000_000).unwrap();
    assert_eq!((order.shot, order.total, order.placements), (74, 172, 26));
    assert_eq!(last::after(Bitboard(open), 2, 64, 1_000_000), Some(175));
    // The search over listed placements gets there only with more time...
    assert_eq!(total(without, sunk, 4, open), (175, 26));
    assert_eq!(total(patient, sunk, 4, open), (172, 26));
    // ...the engine, knowing the order outright, at once.
    assert_eq!(total(Broadside::new, sunk, 4, open), (172, 26));
}

/// Longer ships: the engine plays out what the exact order promises, and a
/// patient search over the listed placements finds nothing better.
#[test]
fn longer_ships_last() {
    let mut rng = Rng::new(5);
    // The other ships sunk along the bottom rows, out of the way.
    let rows = |r: usize, len: usize| (0..len).fold(0u128, |m, c| m | 1 << (r * 10 + c));
    let mut checked = 0;
    while checked < 12 {
        let last = rng.below(4) as usize;
        let mut sunk = Vec::new();
        for (i, s) in (0..SHIPS).filter(|&s| s != last).enumerate() {
            sunk.push((s, rows(6 + i, LENGTHS[s])));
        }
        let sunk: [(usize, u128); 4] = sunk.try_into().unwrap();
        // Unshot cells scattered over the top five rows.
        let mut open = 0u128;
        for c in 0..50 {
            if rng.below(100) < 55 {
                open |= 1 << c;
            }
        }
        let Some(order) = last::best(Bitboard(open), LENGTHS[last], 1_000_000) else {
            continue;
        };
        let want = order.total;
        let (got, places) = total(Broadside::new, sunk, last, open);
        if places > 40 {
            continue;
        }
        assert_eq!(got, want, "ship {last}, open {open:x}");
        assert_eq!(
            total(patient, sunk, last, open).0,
            want,
            "ship {last}, open {open:x}"
        );
        checked += 1;
    }
}

/// How much the exact order is worth over whole games: the engine without
/// it plays on, and wherever the last ship is not yet hit, what its shot
/// gives away against the best one is added up (in expected shots).
/// `LAST_GAMES=N LAST_BUDGET=N cargo test --release --test last_ship -- --ignored --nocapture`
#[test]
#[ignore]
fn what_the_exact_order_is_worth() {
    let env = |name: &str, or: u64| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(or)
    };
    let (games, budget) = (env("LAST_GAMES", 100), env("LAST_BUDGET", 2_000));
    let started = std::time::Instant::now();
    let mut rng = Rng::new(77);
    let (mut positions, mut unsolved, mut lost, mut worse) = (0u32, 0u32, 0.0f64, 0u32);
    let mut opened: Vec<u64> = Vec::new();
    let mut by_ship = [(0u32, 0.0f64); SHIPS];
    for _ in 0..games {
        let mut board = Board::new(Layout::random(&mut rng));
        let mut k = Knowledge::new();
        let mut e = without();
        e.search.exact_limit = 3000;
        while !k.won() {
            let c = e.choose(&k, &mut rng);
            let afloat = k.afloat_mask();
            if afloat.count_ones() == 1 {
                let ship = afloat.trailing_zeros() as usize;
                let len = LENGTHS[ship];
                if k.hits.count() as usize + len == 17 {
                    positions += 1;
                    match last::best(k.unknown(), len, budget) {
                        Some(order) => {
                            opened.push(order.opened);
                            let q = last::after(k.unknown(), len, c, 10 * budget)
                                .unwrap_or(order.total);
                            let gap = (q - order.total) as f64 / order.placements as f64;
                            lost += gap;
                            worse += (q > order.total) as u32;
                            by_ship[ship].0 += 1;
                            by_ship[ship].1 += gap;
                        }
                        None => unsolved += 1,
                    }
                }
            }
            k.record(c, board.fire(c));
        }
    }
    opened.sort_unstable();
    let at = |q: f64| opened[((opened.len() - 1) as f64 * q) as usize];
    println!(
        "{games} games, budget {budget}, {:.1} s",
        started.elapsed().as_secs_f64()
    );
    println!("{positions} positions with the last ship not yet hit, {unsolved} not solved");
    println!(
        "positions opened: median {}, 90% {}, 99% {}, most {}",
        at(0.5),
        at(0.9),
        at(0.99),
        at(1.0)
    );
    println!(
        "above 2,000: {}, above 20,000: {}",
        opened.iter().filter(|&&n| n > 2_000).count(),
        opened.iter().filter(|&&n| n > 20_000).count()
    );
    println!(
        "shots not the best: {worse}; expected shots given away per game: {:.5}",
        lost / games as f64
    );
    for (s, (n, l)) in by_ship.iter().enumerate() {
        println!(
            "  ship {s}: {n} positions, {:.5} shots per game",
            l / games as f64
        );
    }
}
