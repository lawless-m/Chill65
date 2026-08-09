//! Assembler front end for the Atari coin-op assembler dialect.
//!
//! The input is MACRO-11-shaped source with the 6502 instruction set supplied as
//! a macro package. The authoritative specification for the dialect is Atari's
//! own toolchain, archived at `historicalsource/atari-coin-op-assembler` under
//! `atari_tools/` — see `frontend.md`.
//!
//! Pipeline, in the order source flows through it:
//!
//! ```text
//! source -> lexer -> macros -> directives -+-> encode -> assemble -> image
//!                                  ^       |
//!                                  +- expr -+
//! ```
//!
//! Nothing here is implemented yet; each module carries the responsibility it
//! will take on and the task that fills it in.

pub mod assemble;
pub mod directives;
pub mod encode;
pub mod expr;
pub mod lexer;
pub mod macros;
