//! Turning "diverged at frame N" into "diverged at frame N, in routine X".
//!
//! The method is direct. Find the first frame where two runs disagree, take
//! both pictures at that frame, and for every pixel that differs ask the write
//! log which instruction last wrote it. Map those addresses to the nearest
//! preceding symbol and count. The routine at the top of the list is the one
//! that drew the difference.
//!
//! Two things this handles that a naive version would not:
//!
//! - **The bitmap is never cleared.** A pixel differing at frame 400 may have
//!   been drawn at frame 12 and untouched since, so the log runs for the whole
//!   run and remembers the last writer however long ago that was.
//! - **Both sides can be blamed.** Under fault injection the two runs are both
//!   ours, and the interesting answer may be "the clean side drew something
//!   here that the faulty side never did". Attribution merges both.
//!
//! When the counterparty is MAME or a verilated core there is no log to merge —
//! attribution comes from our side alone, which still names the routine that
//! drew *our* version of the disputed pixels.

use std::collections::HashMap;
use std::fmt;

use crate::diverge::{compare, Comparison};
use crate::ours::OurRuntime;
use crate::reference::Reference;
use crate::symbols::Symbols;
use crate::trace::Trace;

/// One routine's share of the blame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blame {
    /// The symbol, rendered as `NAME [unit]`.
    pub symbol: String,
    /// Differing pixels this routine last wrote.
    pub pixels: usize,
}

/// What localisation concluded.
#[derive(Debug, Clone)]
pub struct Report {
    /// The underlying stream comparison.
    pub comparison: Comparison,
    /// Pixels that differ at the divergent frame.
    pub differing: usize,
    /// Differing pixels no side could attribute — never written since the log
    /// began, so nothing in the run is responsible for them.
    pub unattributed: usize,
    /// Routines ranked by how many differing pixels they last wrote.
    pub blame: Vec<Blame>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(frame) = self.comparison.first_divergence else {
            return write!(f, "{}", self.comparison);
        };
        write!(f, "diverged at frame {frame}: ")?;
        if self.blame.is_empty() {
            write!(f, "{} pixels differ, none attributable", self.differing)?;
        } else {
            let parts: Vec<String> = self
                .blame
                .iter()
                .map(|b| format!("{} ({} pixels)", b.symbol, b.pixels))
                .collect();
            write!(f, "{}", parts.join(", "))?;
        }
        if self.unattributed > 0 {
            write!(f, ", {} unattributed", self.unattributed)?;
        }
        Ok(())
    }
}

/// Blame the routines behind the pixels where two pictures differ.
///
/// `writers` carries one entry per side that can attribute — two under fault
/// injection, one when the counterparty is external.
pub fn attribute(
    rgb_a: &[u8],
    rgb_b: &[u8],
    writers: &[&[Option<u16>]],
    symbols: &Symbols,
) -> (usize, usize, Vec<Blame>) {
    let pixels = rgb_a.len() / 3;
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut differing = 0;
    let mut unattributed = 0;

    for p in 0..pixels {
        let range = p * 3..p * 3 + 3;
        if rgb_a[range.clone()] == rgb_b[range] {
            continue;
        }
        differing += 1;

        // A pixel is credited to each side that wrote it, but only once per
        // side, so a routine cannot be blamed twice for the same pixel.
        let mut blamed = Vec::new();
        for side in writers {
            if let Some(pc) = side.get(p).copied().flatten() {
                let name = symbols.routine_for(pc);
                if !blamed.contains(&name) {
                    *counts.entry(name.clone()).or_default() += 1;
                    blamed.push(name);
                }
            }
        }
        if blamed.is_empty() {
            unattributed += 1;
        }
    }

    let mut blame: Vec<Blame> = counts
        .into_iter()
        .map(|(symbol, pixels)| Blame { symbol, pixels })
        .collect();
    // Most pixels first; ties broken by name so the report is reproducible.
    blame.sort_by(|a, b| b.pixels.cmp(&a.pixels).then_with(|| a.symbol.cmp(&b.symbol)));
    (differing, unattributed, blame)
}

/// Compare two of our own runtimes over a trace and localise any divergence.
pub fn localise(
    a: &mut OurRuntime,
    b: &mut OurRuntime,
    trace: &Trace,
    frames: u32,
    symbols: &Symbols,
) -> Result<Report, String> {
    let comparison = compare(&a.hashes(trace, frames)?, &b.hashes(trace, frames)?);

    let Some(frame) = comparison.first_divergence else {
        return Ok(Report {
            comparison,
            differing: 0,
            unattributed: 0,
            blame: Vec::new(),
        });
    };

    // Re-run only as far as the divergence, this time keeping pixels and the
    // write log. Carrying those for every frame of a long run would cost
    // hundreds of megabytes to no purpose.
    let sa = a.snapshot(trace, frame as u32)?;
    let sb = b.snapshot(trace, frame as u32)?;

    let (differing, unattributed, blame) = attribute(
        &sa.rgb,
        &sb.rgb,
        &[&sa.writers, &sb.writers],
        symbols,
    );
    Ok(Report {
        comparison,
        differing,
        unattributed,
        blame,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols() -> Symbols {
        Symbols::parse("A000 START [global]\nB000 DRAW [CDB.MAC]\n").expect("parse")
    }

    /// Three pixels: 0 and 2 differ, 1 agrees.
    fn pictures() -> (Vec<u8>, Vec<u8>) {
        (vec![1, 1, 1, 2, 2, 2, 3, 3, 3], vec![9, 9, 9, 2, 2, 2, 8, 8, 8])
    }

    #[test]
    fn blames_the_routine_that_wrote_the_differing_pixels() {
        let (a, b) = pictures();
        let writers = [Some(0xB004), Some(0xA000), Some(0xB004)];
        let (differing, unattributed, blame) =
            attribute(&a, &b, &[&writers], &symbols());

        assert_eq!(differing, 2, "pixels 0 and 2");
        assert_eq!(unattributed, 0);
        assert_eq!(blame.len(), 1, "both differing pixels came from DRAW");
        assert_eq!(blame[0].symbol, "DRAW [CDB.MAC]");
        assert_eq!(blame[0].pixels, 2);
        // Pixel 1 agrees, so START is not blamed despite having written it.
    }

    #[test]
    fn merges_both_sides_without_double_counting() {
        let (a, b) = pictures();
        let side_a = [Some(0xB000), None, Some(0xB000)];
        let side_b = [Some(0xB000), None, Some(0xA000)];
        let (differing, unattributed, blame) =
            attribute(&a, &b, &[&side_a, &side_b], &symbols());

        assert_eq!(differing, 2);
        assert_eq!(unattributed, 0);
        // Pixel 0: both sides say DRAW — counted once. Pixel 2: DRAW and START.
        let draw = blame.iter().find(|b| b.symbol == "DRAW [CDB.MAC]").unwrap();
        assert_eq!(draw.pixels, 2);
        let start = blame.iter().find(|b| b.symbol == "START [global]").unwrap();
        assert_eq!(start.pixels, 1);
    }

    #[test]
    fn unwritten_pixels_are_counted_not_invented() {
        let (a, b) = pictures();
        let writers = [None, None, None];
        let (differing, unattributed, blame) =
            attribute(&a, &b, &[&writers], &symbols());

        assert_eq!(differing, 2);
        assert_eq!(unattributed, 2);
        assert!(blame.is_empty(), "nothing may be blamed on nothing");
    }

    #[test]
    fn ranking_is_by_pixel_count_then_name() {
        let a = vec![0; 12];
        let b = vec![1; 12];
        let writers = [Some(0xB000), Some(0xB000), Some(0xA000), Some(0xA000)];
        let (_, _, blame) = attribute(&a, &b, &[&writers], &symbols());
        assert_eq!(blame.len(), 2);
        assert_eq!(blame[0].pixels, 2);
        assert_eq!(blame[1].pixels, 2);
        // Equal counts, so alphabetical — a stable, reproducible order.
        assert_eq!(blame[0].symbol, "DRAW [CDB.MAC]");
        assert_eq!(blame[1].symbol, "START [global]");
    }
}
