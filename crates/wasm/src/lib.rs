//! The engine for the browser, one game against a human at a time.
//!
//! A plain C ABI over WebAssembly, so the page needs no glue code: every
//! call takes and returns numbers. Cells are `row * 10 + col`; ships are
//! indexed Carrier, Battleship, Cruiser, Submarine, Destroyer; placement slots
//! are `top-left cell * 2 + vertical`. Outcomes: 0 miss, 1 hit, 2 + ship sunk.
//!
//! The engine and its placer persist across games, so over a session it
//! learns where the player likes to put ships and where they shoot.

use broadside_core::ai::engine::Broadside;
use broadside_core::ai::{self, Placer, Shooter};
use broadside_core::{Board, Knowledge, Layout, Outcome, Rng};
use std::cell::RefCell;

struct Session {
    shooter: Broadside,
    placer: Box<dyn Placer>,
    rng: Rng,
    /// The player's fleet once placed, and a random one offered to them.
    fleet: Option<Layout>,
    offer: Layout,
    engine_fleet: Layout,
    /// The engine's fleet as the player shoots it, and the player's as the
    /// engine does; what each side knows.
    their_board: Board,
    our_board: Option<Board>,
    player: Knowledge,
    engine: Knowledge,
    player_shots: Vec<usize>,
    heat: [f32; 100],
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

fn with<T>(f: impl FnOnce(&mut Session) -> T) -> T {
    SESSION.with(|s| f(s.borrow_mut().as_mut().expect("bs_new_game first")))
}

fn code(o: Outcome) -> i32 {
    match o {
        Outcome::Miss => 0,
        Outcome::Hit => 1,
        Outcome::Sunk(s) => 2 + s.index() as i32,
    }
}

/// Starts a game (the first call also starts the session). The engine
/// places its fleet; the player places theirs with `bs_set_fleet`.
#[unsafe(no_mangle)]
pub extern "C" fn bs_new_game(seed: f64) {
    SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let session = s.get_or_insert_with(|| {
            let mut rng = Rng::new(seed as u64);
            let offer = Layout::random(&mut rng);
            Session {
                shooter: Broadside::new(),
                placer: ai::placer("broadside").expect("built-in placer"),
                rng,
                fleet: None,
                offer,
                engine_fleet: offer,
                their_board: Board::new(offer),
                our_board: None,
                player: Knowledge::new(),
                engine: Knowledge::new(),
                player_shots: Vec::new(),
                heat: [0.0; 100],
            }
        });
        session.engine_fleet = session.placer.place(&mut session.rng);
        session.their_board = Board::new(session.engine_fleet);
        session.fleet = None;
        session.our_board = None;
        session.player = Knowledge::new();
        session.engine = Knowledge::new();
        session.player_shots.clear();
        session.offer = Layout::random(&mut session.rng);
    });
}

/// Draws a new random fleet to offer the player.
#[unsafe(no_mangle)]
pub extern "C" fn bs_random_fleet() {
    with(|s| s.offer = Layout::random(&mut s.rng));
}

/// Slot of ship `ship` in the offered random fleet.
#[unsafe(no_mangle)]
pub extern "C" fn bs_offer_slot(ship: i32) -> i32 {
    with(|s| s.offer.slots[ship as usize] as i32)
}

/// The player's fleet by slot per ship: 1 if legal (and taken), else 0.
#[unsafe(no_mangle)]
pub extern "C" fn bs_set_fleet(a: i32, b: i32, c: i32, d: i32, e: i32) -> i32 {
    let slots = [a, b, c, d, e].map(|x| x.clamp(0, 255) as u8);
    let Some(layout) = Layout::new(slots) else {
        return 0;
    };
    with(|s| {
        s.fleet = Some(layout);
        s.our_board = Some(Board::new(layout));
    });
    1
}

/// The player fires at `cell` of the engine's fleet: the outcome, or -1 if
/// the cell is off the board or already shot.
#[unsafe(no_mangle)]
pub extern "C" fn bs_player_fire(cell: i32) -> i32 {
    with(|s| {
        if !(0..100).contains(&cell) || s.player.shots().has(cell as usize) {
            return -1;
        }
        let c = cell as usize;
        let o = s.their_board.fire(c);
        s.player.record(c, o);
        s.player_shots.push(c);
        code(o)
    })
}

/// The engine fires: `cell * 16 + outcome`, or -1 before the player's
/// fleet is placed or after the engine has won.
#[unsafe(no_mangle)]
pub extern "C" fn bs_engine_fire() -> i32 {
    with(|s| {
        let Some(board) = s.our_board.as_mut() else {
            return -1;
        };
        if s.engine.won() {
            return -1;
        }
        // The player fires next: the engine knows how close they are.
        s.shooter.race(&s.engine_fleet, &s.player);
        let c = s.shooter.choose(&s.engine, &mut s.rng);
        let o = board.fire(c);
        s.engine.record(c, o);
        c as i32 * 16 + code(o)
    })
}

/// 1 once the player has sunk the engine's whole fleet.
#[unsafe(no_mangle)]
pub extern "C" fn bs_player_won() -> i32 {
    with(|s| s.player.won() as i32)
}

/// 1 once the engine has sunk the player's whole fleet.
#[unsafe(no_mangle)]
pub extern "C" fn bs_engine_won() -> i32 {
    with(|s| s.engine.won() as i32)
}

/// Slot of the engine's ship `ship`: for the reveal once the game is over
/// (-1 before then).
#[unsafe(no_mangle)]
pub extern "C" fn bs_engine_slot(ship: i32) -> i32 {
    with(|s| {
        if s.player.won() || s.engine.won() {
            s.engine_fleet.slots[ship as usize] as i32
        } else {
            -1
        }
    })
}

/// After a game: both sides of the engine learn from it.
#[unsafe(no_mangle)]
pub extern "C" fn bs_finish() {
    with(|s| {
        if let Some(fleet) = s.fleet {
            s.shooter.learn(&fleet);
        }
        s.placer.learn(&s.engine_fleet, &s.player_shots);
    });
}

/// Recomputes the engine's belief about the player's fleet; read it with
/// `bs_heat`.
#[unsafe(no_mangle)]
pub extern "C" fn bs_update_heat() {
    with(|s| {
        let chances = s.shooter.chances(&s.engine);
        for (h, c) in s.heat.iter_mut().zip(chances) {
            *h = c as f32;
        }
    });
}

/// What the engine expects of the player's next fleet: the chance that its
/// ships all lie apart (`kind` 0), that some meet at a corner but none
/// share an edge (1), or that some share an edge (2).
#[unsafe(no_mangle)]
pub extern "C" fn bs_spacing(kind: i32) -> f32 {
    with(|s| s.shooter.habits.belief()[kind as usize] as f32)
}

/// The chance, as the engine sees it, that `cell` of the player's fleet
/// holds a ship.
#[unsafe(no_mangle)]
pub extern "C" fn bs_heat(cell: i32) -> f32 {
    with(|s| s.heat[cell as usize])
}
