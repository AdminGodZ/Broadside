//! Broadside core: pure game logic and AI. No I/O lives here, so the same code
//! drives the CLI, the simulator and (later) a WebAssembly build.

pub mod ai;
pub mod bitboard;
pub mod fleet;
pub mod game;
pub mod posterior;
pub mod rng;
pub mod sim;
pub mod symmetry;

pub use bitboard::Bitboard;
pub use fleet::{FLEET, Ship};
pub use game::{Board, Knowledge, Layout, Outcome};
pub use posterior::Posterior;
pub use rng::Rng;
