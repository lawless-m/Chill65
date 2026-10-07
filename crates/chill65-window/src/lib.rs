//! Space Duel or Tempest in a window, drawn by the Trexy tube renderer.
//!
//! Chill65 runs the machine; Trexy models the tube. This crate is the joint
//! between them, and it lives here rather than in Trexy because the renderer is
//! general — its own architecture notes call this project "External; adapter
//! only" — while the adapter is game-shaped by nature.
//!
//! Which game is a property of the corpus, not a flag: [`machine::Game::detect`]
//! reads the source directory and everything downstream — frame rate, full-scale
//! deflection, colour, the panel — follows from what it finds.
//!
//! It is **outside the Chill65 workspace** on purpose. It path-depends on the
//! sibling Trexy checkout, and a workspace member's manifest is read by every
//! `cargo build` at the root, so including it would make the whole repository
//! unbuildable for anyone without Trexy beside it. Build it from its own
//! directory instead:
//!
//! ```text
//! cd crates/chill65-window
//! CHILL65_CORPUS=/path/to/space-duel cargo run
//! CHILL65_CORPUS=/path/to/tempest    cargo run
//! ```
//!
//! # Layout
//!
//! - [`machine`] — the machines, the controls, and a frame's beam as samples.
//!   No GPU, so it is testable on its own.
//! - [`capture`] — a scripted game written to a trace file, checking itself
//!   against an idle machine. This is how the whole path is verified with
//!   nobody at the keyboard.
//! - [`live`] — the machine on its own thread, paced to the wall clock and
//!   filling a ring buffer the window drains.
//! - [`window`] — the winit application and the blit that puts the tube on
//!   screen.

pub mod capture;
pub mod live;
pub mod machine;
pub mod window;
