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
}
