//! Compiled Crystal Castles routines, running beside the interpreter.
//!
//! Plan §5's creep line made concrete. Routines named in `routines.txt` are
//! lowered to Rust at build time and dispatched at their entry addresses; every
//! other instruction is interpreted, and the two share one `Machine`.
//!
//! # What is and is not in this crate
//!
//! The manifest holds routine **names**. Those are our own reading of the
//! source and are committed. The Rust generated *for* them is not: it is
//! emitted into `OUT_DIR` at build time and never committed, being as
//! game-derived as the ROM itself (plan §9).
//!
//! With no `CHILL65_CORPUS`, the game registry generates empty and everything
//! still builds and tests — which is why the fixture exists.

/// Routines compiled from the committed fixture program. Always present.
///
/// Generated code is allowed to be untidy: an unconditional control transfer
/// makes everything after it in the routine unreachable, which is correct and
/// not worth contorting the emitter to avoid.
#[allow(unused_imports, unused_variables, unreachable_code, non_snake_case, clippy::all)]
pub mod fixture {
    include!(concat!(env!("OUT_DIR"), "/fixture_compiled.rs"));

    /// The fixture's assembled program image, `A000`–`FFFF`.
    pub const IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fixture.bin"));
}

/// Routines compiled from the game. Empty without a corpus.
#[allow(unused_imports, unused_variables, unreachable_code, non_snake_case, clippy::all)]
pub mod game {
    include!(concat!(env!("OUT_DIR"), "/game_compiled.rs"));
}

use chill65_runtime::frame::Compiled;
use chill65_runtime::{Cpu, Machine};

/// A table of compiled routines, dispatched by entry address.
///
/// Dispatch happens only at entries, which is what makes the yield rule in
/// [`Compiled`] safe: a routine that stops early is *finished by the
/// interpreter*, never re-entered from the top.
pub struct Registry {
    routines: &'static [(u16, fn(&mut Cpu, &mut Machine, u64) -> u64)],
    executed: u64,
}

impl Registry {
    pub fn new(routines: &'static [(u16, fn(&mut Cpu, &mut Machine, u64) -> u64)]) -> Registry {
        Registry {
            routines,
            executed: 0,
        }
    }

    /// The fixture's routines.
    pub fn fixture() -> Registry {
        Registry::new(fixture::ROUTINES)
    }

    /// The game's routines — empty unless built with a corpus and a non-empty
    /// manifest.
    pub fn game() -> Registry {
        Registry::new(game::ROUTINES)
    }

    /// How many routines are compiled.
    pub fn len(&self) -> usize {
        self.routines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routines.is_empty()
    }
}

impl Compiled for Registry {
    fn enter(&mut self, cpu: &mut Cpu, machine: &mut Machine, deadline: u64) -> bool {
        let pc = cpu.pc;
        let Some((_, f)) = self.routines.iter().find(|(entry, _)| *entry == pc) else {
            return false;
        };
        let ran = f(cpu, machine, deadline);
        self.executed += ran;
        // Reporting `true` after executing nothing would spin: the caller would
        // offer the same pc again forever. A routine that yields immediately
        // hands the instruction back to the interpreter.
        ran > 0
    }

    fn compiled_instructions(&self) -> u64 {
        self.executed
    }
}
