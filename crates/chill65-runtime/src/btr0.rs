//! `BTR0` beam traces — the file the tube renderer reads.
//!
//! A beam trace is a time-ordered, piecewise-linear description of where the
//! beam actually went and how hard it was driven. It is not a picture: it
//! carries no resolution and no aspect ratio, only normalised deflection,
//! linear-light drive and time. What a display makes of that — spot size,
//! phosphor, persistence, glow, geometry — is the renderer's business, and
//! deliberately none of ours.
//!
//! The format is specified normatively by **Trexy TRACE-FORMAT.md v0**
//! (`../Trexy/TRACE-FORMAT.md` beside this repository). This module implements
//! the writer half of it.
//!
//! # Why the bytes are written by hand
//!
//! Trexy ships `beam-trace`, a crate that does exactly this. Depending on it
//! by path would be shorter and would be wrong: this crate takes **no
//! dependencies at all** — see its `Cargo.toml`, and `ccrun`'s hand-written
//! PPM for the same decision made before — so that it targets wasm32 unchanged
//! and so that Chill65 builds on a machine where Trexy has never been cloned.
//! A sibling checkout is not a dependency we can require.
//!
//! The format is 64 bytes of header and 32 bytes per sample. Writing that by
//! hand costs less than the coupling would.
//!
//! # Layout
//!
//! Little-endian throughout. Header:
//!
//! ```text
//! 0   4   magic "BTR0"
//! 4   4   u32  version = 0
//! 8   8   f64  epoch, producer-defined origin
//! 16  8   u64  sample count
//! 24  4   f32  epsilon, the positional bound this trace honours
//! 28  4   f32  nominal refresh Hz, informational; 0 = unknown
//! 32  32  producer id, UTF-8, NUL-padded
//! ```
//!
//! Then one 32-byte record per sample:
//!
//! ```text
//! 0   4   f32  x        normalised deflection, nominally -1..+1, y-up
//! 4   4   f32  y
//! 8   4   f32  drive r  linear light, >= 0, unclamped; 0 = blanked
//! 12  4   f32  drive g
//! 16  4   f32  drive b
//! 20  4   f32  t        seconds since epoch, strictly increasing
//! 24  4   u32  flags    bit 0 = DISCONTINUITY
//! 28  4   u32  reserved = 0
//! ```
//!
//! # Refusing rather than repairing
//!
//! [`Trace::push`] rejects exactly what the renderer's loader rejects:
//! non-finite floats, negative drive, and a timestamp that does not advance.
//! A trace that would be refused at the far end is refused here, where the
//! offending sample can still be named.

use std::path::Path;

/// Magic at offset 0.
pub const MAGIC: [u8; 4] = *b"BTR0";

/// The format version this module writes.
pub const VERSION: u32 = 0;

/// Header length in bytes.
pub const HEADER_LEN: usize = 64;

/// Length of one sample record in bytes.
pub const SAMPLE_LEN: usize = 32;

/// The producer id field is 32 bytes, NUL-padded.
pub const PRODUCER_ID_LEN: usize = 32;

/// This sample is not path-continuous with the one before it. The renderer
/// deposits nothing across the gap.
pub const DISCONTINUITY: u32 = 1 << 0;

/// The spec's default positional bound: a quarter of a deposit texel at the
/// renderer's 2x supersampling.
pub const DEFAULT_EPSILON: f32 = 1.0 / 4096.0;

/// Why a sample was refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TraceError {
    /// The producer id does not fit the fixed 32-byte field.
    ProducerIdTooLong(usize),
    /// A coordinate, drive or timestamp was NaN or infinite.
    NotFinite(&'static str),
    /// Drive is radiant power: it cannot be negative.
    NegativeDrive(f32),
    /// Timestamps must strictly increase within a trace.
    TimeNotIncreasing { previous: f32, pushed: f32 },
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TraceError::ProducerIdTooLong(n) => {
                write!(f, "producer id is {n} bytes, the field is {PRODUCER_ID_LEN}")
            }
            TraceError::NotFinite(field) => write!(f, "{field} is not finite"),
            TraceError::NegativeDrive(v) => write!(f, "drive {v} is negative"),
            TraceError::TimeNotIncreasing { previous, pushed } => {
                write!(f, "t must strictly increase, {pushed} follows {previous}")
            }
        }
    }
}

/// A trace under construction. Samples are encoded as they arrive.
#[derive(Clone, Debug)]
pub struct Trace {
    /// Producer-defined time origin. Zero unless something needs otherwise.
    pub epoch: f64,
    epsilon: f32,
    refresh_hz: f32,
    producer: String,
    records: Vec<u8>,
    count: u64,
    last_t: Option<f32>,
}

impl Trace {
    /// A new trace from a producer id, a positional bound and the nominal
    /// refresh rate of whatever is drawing (0 if there is no such thing).
    pub fn new(producer_id: &str, epsilon: f32, refresh_hz: f32) -> Result<Self, TraceError> {
        let len = producer_id.len();
        if len > PRODUCER_ID_LEN {
            return Err(TraceError::ProducerIdTooLong(len));
        }
        Ok(Trace {
            epoch: 0.0,
            epsilon,
            refresh_hz,
            producer: producer_id.to_owned(),
            records: Vec::new(),
            count: 0,
            last_t: None,
        })
    }

    /// Samples pushed so far.
    pub fn len(&self) -> u64 {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The timestamp of the last sample, if any.
    pub fn last_t(&self) -> Option<f32> {
        self.last_t
    }

    /// Append one sample.
    pub fn push(
        &mut self,
        x: f32,
        y: f32,
        drive: [f32; 3],
        t: f32,
        flags: u32,
    ) -> Result<(), TraceError> {
        for (value, name) in [(x, "x"), (y, "y"), (t, "t")] {
            if !value.is_finite() {
                return Err(TraceError::NotFinite(name));
            }
        }
        for (value, name) in drive.iter().zip(["drive r", "drive g", "drive b"]) {
            if !value.is_finite() {
                return Err(TraceError::NotFinite(name));
            }
            if *value < 0.0 {
                return Err(TraceError::NegativeDrive(*value));
            }
        }
        if let Some(previous) = self.last_t {
            if t <= previous {
                return Err(TraceError::TimeNotIncreasing { previous, pushed: t });
            }
        }

        for value in [x, y, drive[0], drive[1], drive[2], t] {
            self.records.extend_from_slice(&value.to_le_bytes());
        }
        self.records.extend_from_slice(&flags.to_le_bytes());
        self.records.extend_from_slice(&0u32.to_le_bytes());

        self.last_t = Some(t);
        self.count += 1;
        Ok(())
    }

    /// The whole file: header then records.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.records.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.epsilon.to_le_bytes());
        out.extend_from_slice(&self.refresh_hz.to_le_bytes());
        let mut id = [0u8; PRODUCER_ID_LEN];
        id[..self.producer.len()].copy_from_slice(self.producer.as_bytes());
        out.extend_from_slice(&id);
        debug_assert_eq!(out.len(), HEADER_LEN);
        out.extend_from_slice(&self.records);
        out
    }

    /// Write the file.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        std::fs::write(path, self.to_bytes()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le_u32(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }
    fn le_u64(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }
    fn le_f32(b: &[u8], at: usize) -> f32 {
        f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }
    fn le_f64(b: &[u8], at: usize) -> f64 {
        f64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }

    fn trace() -> Trace {
        Trace::new("chill65/test", DEFAULT_EPSILON, 60.0).unwrap()
    }

    #[test]
    fn the_header_is_sixty_four_bytes_of_the_specified_fields() {
        let mut t = trace();
        t.push(0.0, 0.0, [0.0; 3], 0.0, DISCONTINUITY).unwrap();
        t.push(0.5, -0.25, [1.0, 0.0, 0.0], 0.001, 0).unwrap();
        let b = t.to_bytes();

        assert_eq!(&b[0..4], b"BTR0");
        assert_eq!(le_u32(&b, 4), 0);
        assert_eq!(le_f64(&b, 8), 0.0);
        assert_eq!(le_u64(&b, 16), 2);
        assert_eq!(le_f32(&b, 24), DEFAULT_EPSILON);
        assert_eq!(le_f32(&b, 28), 60.0);
        assert_eq!(&b[32..44], b"chill65/test");
        assert!(b[44..64].iter().all(|&c| c == 0), "NUL-padded");
    }

    #[test]
    fn the_file_is_the_header_plus_thirty_two_bytes_a_sample() {
        let mut t = trace();
        assert_eq!(t.to_bytes().len(), HEADER_LEN);
        for i in 0..5 {
            t.push(0.0, 0.0, [0.0; 3], i as f32 * 0.001, 0).unwrap();
        }
        assert_eq!(t.len(), 5);
        assert_eq!(t.to_bytes().len(), HEADER_LEN + 5 * SAMPLE_LEN);
    }

    #[test]
    fn a_record_carries_its_seven_fields_at_the_right_offsets() {
        let mut t = trace();
        t.push(0.25, -0.75, [0.1, 0.2, 0.3], 0.5, DISCONTINUITY)
            .unwrap();
        let b = t.to_bytes();
        let r = HEADER_LEN;

        assert_eq!(le_f32(&b, r), 0.25);
        assert_eq!(le_f32(&b, r + 4), -0.75);
        assert_eq!(le_f32(&b, r + 8), 0.1);
        assert_eq!(le_f32(&b, r + 12), 0.2);
        assert_eq!(le_f32(&b, r + 16), 0.3);
        assert_eq!(le_f32(&b, r + 20), 0.5);
        assert_eq!(le_u32(&b, r + 24), DISCONTINUITY);
        assert_eq!(le_u32(&b, r + 28), 0, "reserved is written zero");
    }

    #[test]
    fn time_must_strictly_increase() {
        let mut t = trace();
        t.push(0.0, 0.0, [0.0; 3], 0.010, 0).unwrap();
        assert_eq!(
            t.push(0.0, 0.0, [0.0; 3], 0.010, 0),
            Err(TraceError::TimeNotIncreasing {
                previous: 0.010,
                pushed: 0.010
            }),
            "equal is not increasing"
        );
        assert_eq!(
            t.push(0.0, 0.0, [0.0; 3], 0.009, 0),
            Err(TraceError::TimeNotIncreasing {
                previous: 0.010,
                pushed: 0.009
            })
        );
        assert_eq!(t.len(), 1, "a refused sample is not written");
        t.push(0.0, 0.0, [0.0; 3], 0.011, 0).unwrap();
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn drive_cannot_be_negative_and_nothing_may_be_infinite() {
        let mut t = trace();
        assert_eq!(
            t.push(0.0, 0.0, [-0.001, 0.0, 0.0], 0.0, 0),
            Err(TraceError::NegativeDrive(-0.001))
        );
        assert_eq!(
            t.push(f32::NAN, 0.0, [0.0; 3], 0.0, 0),
            Err(TraceError::NotFinite("x"))
        );
        assert_eq!(
            t.push(0.0, f32::INFINITY, [0.0; 3], 0.0, 0),
            Err(TraceError::NotFinite("y"))
        );
        assert_eq!(
            t.push(0.0, 0.0, [0.0; 3], f32::NAN, 0),
            Err(TraceError::NotFinite("t"))
        );
        assert_eq!(
            t.push(0.0, 0.0, [0.0, f32::NAN, 0.0], 0.0, 0),
            Err(TraceError::NotFinite("drive g"))
        );
        assert!(t.is_empty());
        // Overdrive above nominal is legal and meaningful — the renderer's
        // saturation model wants it (TRACE-FORMAT.md v0 §1).
        t.push(0.0, 0.0, [3.0, 3.0, 3.0], 0.0, 0).unwrap();
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn a_producer_id_longer_than_the_field_is_refused() {
        let long = "x".repeat(PRODUCER_ID_LEN + 1);
        assert_eq!(
            Trace::new(&long, DEFAULT_EPSILON, 0.0).err(),
            Some(TraceError::ProducerIdTooLong(PRODUCER_ID_LEN + 1))
        );
        let exact = "y".repeat(PRODUCER_ID_LEN);
        let t = Trace::new(&exact, DEFAULT_EPSILON, 0.0).unwrap();
        let b = t.to_bytes();
        assert_eq!(&b[32..64], exact.as_bytes(), "a full field has no NUL");
    }
}
