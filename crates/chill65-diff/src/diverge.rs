//! Comparing two per-frame hash streams, and the on-disk form of a stream.
//!
//! The comparison is deliberately blunt: the first frame where two runs stop
//! agreeing. Everything after it is downstream of that one divergence, so
//! reporting a hundred differing frames would be reporting the same fact a
//! hundred times.
//!
//! Streams of unequal length are compared over their common prefix and the
//! mismatch is **stated**, never silently truncated — two runs that produced
//! different numbers of frames have already gone wrong somewhere, and hiding it
//! behind a clean "identical" would be the worst possible answer.

use std::fmt;
use std::path::Path;

/// What comparing two streams found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comparison {
    /// How many frames were actually compared — the shorter of the two.
    pub compared: usize,
    /// The first frame where they differ, or `None` if the common prefix agrees.
    pub first_divergence: Option<usize>,
    /// Set only when the streams are different lengths: `(a, b)`.
    pub lengths: Option<(usize, usize)>,
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.first_divergence {
            Some(frame) => write!(f, "diverged at frame {frame}")?,
            None => write!(f, "identical over {} frames", self.compared)?,
        }
        if let Some((a, b)) = self.lengths {
            write!(f, " (lengths differ: {a} and {b})")?;
        }
        Ok(())
    }
}

/// Compare two hash streams over their common prefix.
pub fn compare(a: &[u64], b: &[u64]) -> Comparison {
    let compared = a.len().min(b.len());
    Comparison {
        compared,
        first_divergence: a.iter().zip(b).position(|(x, y)| x != y),
        lengths: (a.len() != b.len()).then_some((a.len(), b.len())),
    }
}

/// Compare two hash streams with `b` shifted by `k` frames: `a[n]` against
/// `b[n + k]`.
///
/// `k` is a **measured** quantity, not a search parameter. Frames carry the
/// emulated cycle they were captured at, so the shift between two streams is
/// read off with [`whole_frame_offset`] rather than fitted by trying values —
/// `harness.md` §12 records a shift search that could not discriminate, which
/// is what motivated dating the frames in the first place.
///
/// Divergence is reported in **`a`'s frame numbers**, since `a` is ours and
/// that is the numbering the write log and routine attribution use.
pub fn compare_offset(a: &[u64], b: &[u64], k: i64) -> Comparison {
    // The overlap: indices n where both a[n] and b[n+k] exist.
    let lo = (-k).max(0) as usize;
    let hi = a.len().min((b.len() as i64 - k).max(0) as usize);
    let compared = hi.saturating_sub(lo);

    let first_divergence = (lo..hi).find(|&n| a[n] != b[(n as i64 + k) as usize]);

    Comparison {
        compared,
        first_divergence,
        lengths: (a.len() != b.len()).then_some((a.len(), b.len())),
    }
}

/// The whole-frame shift between two streams of capture stamps.
///
/// Both sides date every frame in emulated CPU cycles, so `theirs[n] -
/// ours[n]` divided by the frame period is how many frames apart the two
/// numberings are. Taken at the **start** of the run, where neither side has
/// accumulated any drift.
///
/// **Nearest, not floor.** A difference of −2 cycles is not "one frame behind";
/// flooring would say so, and the caller turns this into an applied shift.
///
/// Returns `None` when either side is unstamped or empty, which means compare
/// by index and say so — never guess.
pub fn whole_frame_offset(ours: &[u64], theirs: &[u64], period: u64) -> Option<i64> {
    let (a, b) = (*ours.first()?, *theirs.first()?);
    let delta = b as i64 - a as i64;
    Some((delta as f64 / period as f64).round() as i64)
}

/// Render a stream as text: one 16-hex-digit hash per line.
pub fn write_stream(path: &Path, hashes: &[u64]) -> Result<(), String> {
    let mut text = String::with_capacity(hashes.len() * 17);
    for hash in hashes {
        text.push_str(&format!("{hash:016x}\n"));
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Read a stream written by [`write_stream`]. Blank lines are skipped.
pub fn read_stream(path: &Path) -> Result<Vec<u64>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hashes = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let hash = u64::from_str_radix(line, 16)
            .map_err(|_| format!("{}:{}: {line:?} is not a 16-digit hex hash", path.display(), i + 1))?;
        hashes.push(hash);
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_streams() {
        let c = compare(&[1, 2, 3], &[1, 2, 3]);
        assert_eq!(c.first_divergence, None);
        assert_eq!(c.compared, 3);
        assert_eq!(c.lengths, None);
        assert_eq!(c.to_string(), "identical over 3 frames");
    }

    #[test]
    fn differ_at_the_first_frame() {
        let c = compare(&[9, 2, 3], &[1, 2, 3]);
        assert_eq!(c.first_divergence, Some(0));
        assert_eq!(c.to_string(), "diverged at frame 0");
    }

    #[test]
    fn differ_mid_stream() {
        let c = compare(&[1, 2, 3, 4], &[1, 2, 9, 4]);
        assert_eq!(c.first_divergence, Some(2));
    }

    #[test]
    fn differ_at_the_last_frame() {
        let c = compare(&[1, 2, 3], &[1, 2, 9]);
        assert_eq!(c.first_divergence, Some(2));
    }

    #[test]
    fn length_mismatch_is_stated_not_hidden() {
        // Agreeing prefix, different lengths: not "identical" full stop.
        let c = compare(&[1, 2, 3], &[1, 2]);
        assert_eq!(c.first_divergence, None);
        assert_eq!(c.compared, 2);
        assert_eq!(c.lengths, Some((3, 2)));
        assert_eq!(c.to_string(), "identical over 2 frames (lengths differ: 3 and 2)");

        // And a divergence inside the common prefix still leads.
        let c = compare(&[1, 9], &[1, 2, 3]);
        assert_eq!(c.first_divergence, Some(1));
        assert_eq!(c.to_string(), "diverged at frame 1 (lengths differ: 2 and 3)");
    }

    #[test]
    fn empty_streams() {
        let c = compare(&[], &[]);
        assert_eq!(c.first_divergence, None);
        assert_eq!(c.compared, 0);
        assert_eq!(c.to_string(), "identical over 0 frames");

        let c = compare(&[], &[1]);
        assert_eq!(c.first_divergence, None);
        assert_eq!(c.lengths, Some((0, 1)));
    }

    /// A stream against its own shift agrees at the matching offset and only
    /// there — which is what makes a measured `k` worth applying.
    #[test]
    fn a_shifted_stream_matches_at_its_own_offset() {
        let a = [10u64, 11, 12, 13, 14, 15];
        // b is a delayed by two frames: b[n + 2] == a[n].
        let b = [98u64, 99, 10, 11, 12, 13, 14, 15];

        let aligned = compare_offset(&a, &b, 2);
        assert_eq!(aligned.first_divergence, None, "{aligned:?}");
        assert_eq!(aligned.compared, 6);

        for wrong in [-1i64, 0, 1, 3] {
            let c = compare_offset(&a, &b, wrong);
            assert!(
                c.first_divergence.is_some(),
                "offset {wrong} should not align: {c:?}"
            );
        }
    }

    #[test]
    fn a_negative_offset_reaches_back() {
        // b runs two frames ahead of a: b[n - 2] == a[n].
        let a = [0u64, 0, 10, 11, 12];
        let b = [10u64, 11, 12];
        let c = compare_offset(&a, &b, -2);
        assert_eq!(c.first_divergence, None, "{c:?}");
        assert_eq!(c.compared, 3);
    }

    #[test]
    fn divergence_is_reported_in_our_frame_numbers() {
        let a = [1u64, 2, 3, 4];
        let b = [0u64, 1, 2, 9, 4];
        // b is a delayed by one; b[3] should be a[2] == 3 but is 9.
        let c = compare_offset(&a, &b, 1);
        assert_eq!(c.first_divergence, Some(2));
    }

    #[test]
    fn the_offset_is_measured_from_the_stamps() {
        const PERIOD: u64 = 20_480;
        // Same epoch: zero frames apart.
        assert_eq!(whole_frame_offset(&[20_480], &[20_480], PERIOD), Some(0));
        // A sub-frame lead is still zero frames -- the core starts 336 cycles
        // late and that must not read as a shift.
        assert_eq!(whole_frame_offset(&[20_480], &[20_816], PERIOD), Some(0));
        // And a couple of cycles behind is zero, not minus one.
        assert_eq!(whole_frame_offset(&[20_480], &[20_478], PERIOD), Some(0));
        // A genuine frame of lead reads as one.
        assert_eq!(whole_frame_offset(&[20_480], &[40_960], PERIOD), Some(1));
        // Unstamped or empty: say nothing.
        assert_eq!(whole_frame_offset(&[], &[20_480], PERIOD), None);
    }
}
