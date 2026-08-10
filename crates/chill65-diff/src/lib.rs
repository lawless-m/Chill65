//! Phase 3 of the project: the differential harness.
//!
//! The harness drives our runtime and one or more external reference
//! implementations from an identical recorded input trace, compares their
//! pictures frame by frame, and reports where they first disagree.
//!
//! # It is a debugger, not a certifier
//!
//! Plan §Phase 3: *"the useful output is 'diverged at frame 4,812, routine X'."*
//! Divergence against an external oracle is **expected** — `gate2.md` records
//! six hardware claims still marked UNVERIFIED, and motion objects are not
//! modelled at all. So nothing here asserts that two implementations agree; it
//! asserts that when they disagree we can say precisely where, and get the same
//! answer twice.
//!
//! # Layout
//!
//! - [`trace`] — the recorded input format: per-frame trackball deltas and
//!   switch levels, and the parser and serialiser for it.
//! - [`reference`] — the oracle seam and the canonical frame every
//!   implementation is compared in.
//! - [`ours`] — our own runtime behind that seam.
//! - [`images`] — assembling the ROM images to run, from the corpus.
//!
//! No third-party dependencies, matching the rest of the workspace.

pub mod images;
pub mod ours;
pub mod reference;
pub mod trace;

pub use ours::OurRuntime;
pub use reference::{Frame, Reference};
pub use trace::{FrameInput, Switches, Trace, TraceError};
