//! Recorded input traces — format version 1.
//!
//! A trace is what makes two implementations comparable: both machines get the
//! same trackball movement and the same switch levels on the same frames, so
//! any difference in the resulting picture is theirs, not the input's.
//!
//! # The format
//!
//! UTF-8 text. `#` starts a comment that runs to end of line; blank lines and
//! comment-only lines are ignored everywhere. The first line with any content
//! is the header:
//!
//! ```text
//! chill65-trace 1
//! ```
//!
//! Every line after it describes one frame:
//!
//! ```text
//! <dx> <dy> <switches> [x<count>]
//! ```
//!
//! - `dx`, `dy` — decimal [`i32`] trackball deltas for that frame, positive
//!   meaning right and down. These are handed to
//!   [`chill65_runtime::Input::add_trackball_delta`] unchanged.
//! - `<switches>` — `-` for none, or a comma-separated subset of `Start1`,
//!   `Start2`, `SelfTest`, `Slam`, `CoinAux`, `CoinLeft`, `CoinRight`. Names are
//!   case-sensitive and each may appear once. Listing a switch means it is
//!   **held down** for that frame: these are levels, not edges, so a button
//!   press spanning ten frames appears on all ten.
//! - `x<count>` — optional, repeats that frame line `count` times in total, so
//!   `x600` is six hundred frames and `x1` is the same as omitting it. This is
//!   what keeps a 600-frame idle trace to a single line.
//!
//! The trace's frame count is the expanded line count.
//!
//! ```text
//! # attract mode, no input at all
//! chill65-trace 1
//! 0 0 - x600
//! ```
//!
//! Traces are our own authored work rather than anything game-derived, so
//! unlike ROM images and framebuffers they are committable (plan §9).

use std::fmt;

use chill65_runtime::Switch;

/// The format version this module reads and writes.
pub const VERSION: u32 = 1;

/// The header line introducing a version 1 trace.
pub const HEADER: &str = "chill65-trace 1";

/// Every switch a trace can name, in the order they are serialised.
pub const ALL_SWITCHES: [Switch; 7] = [
    Switch::Start1,
    Switch::Start2,
    Switch::SelfTest,
    Switch::Slam,
    Switch::CoinAux,
    Switch::CoinLeft,
    Switch::CoinRight,
];

/// The name a switch is written as.
pub fn switch_name(switch: Switch) -> &'static str {
    match switch {
        Switch::Start1 => "Start1",
        Switch::Start2 => "Start2",
        Switch::SelfTest => "SelfTest",
        Switch::Slam => "Slam",
        Switch::CoinAux => "CoinAux",
        Switch::CoinLeft => "CoinLeft",
        Switch::CoinRight => "CoinRight",
    }
}

/// The switch a name refers to, or `None` if it names nothing.
pub fn switch_from_name(name: &str) -> Option<Switch> {
    ALL_SWITCHES.into_iter().find(|&s| switch_name(s) == name)
}

/// Which switches are held down for one frame.
///
/// Stored as a bit per switch keyed by [`Switch::bit`], which gives a canonical
/// ordering for free — so serialising and re-parsing yields the same text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Switches(u8);

impl Switches {
    /// No switch held.
    pub const fn none() -> Self {
        Switches(0)
    }

    /// This set with `switch` also held.
    pub fn with(self, switch: Switch) -> Self {
        Switches(self.0 | (1 << switch.bit()))
    }

    /// Is `switch` held?
    pub fn contains(self, switch: Switch) -> bool {
        self.0 & (1 << switch.bit()) != 0
    }

    /// Is nothing held?
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The switches held, in serialisation order.
    pub fn iter(self) -> impl Iterator<Item = Switch> {
        ALL_SWITCHES.into_iter().filter(move |&s| self.contains(s))
    }
}

impl fmt::Display for Switches {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("-");
        }
        for (i, switch) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            f.write_str(switch_name(switch))?;
        }
        Ok(())
    }
}

impl FromIterator<Switch> for Switches {
    fn from_iter<I: IntoIterator<Item = Switch>>(iter: I) -> Self {
        iter.into_iter().fold(Switches::none(), Switches::with)
    }
}

/// One frame's worth of input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameInput {
    /// Horizontal trackball delta, positive right.
    pub dx: i32,
    /// Vertical trackball delta, positive down.
    pub dy: i32,
    /// Switches held down for this frame.
    pub switches: Switches,
}

impl FrameInput {
    /// A frame with no movement and nothing pressed.
    pub const fn idle() -> Self {
        FrameInput {
            dx: 0,
            dy: 0,
            switches: Switches::none(),
        }
    }
}

impl fmt::Display for FrameInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.dx, self.dy, self.switches)
    }
}

/// A recorded trace: one [`FrameInput`] per frame, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Trace {
    pub frames: Vec<FrameInput>,
}

/// Why a trace would not parse. Carries the 1-based source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for TraceError {}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, TraceError> {
    Err(TraceError {
        line,
        message: message.into(),
    })
}

/// Strip a `#` comment and surrounding whitespace.
fn strip(line: &str) -> &str {
    match line.find('#') {
        Some(i) => line[..i].trim(),
        None => line.trim(),
    }
}

impl Trace {
    /// An `n`-frame trace with no input at all.
    pub fn idle(frames: usize) -> Self {
        Trace {
            frames: vec![FrameInput::idle(); frames],
        }
    }

    /// How many frames the trace covers.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Does the trace cover no frames?
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Parse format v1. See the module documentation for the grammar.
    pub fn parse(text: &str) -> Result<Trace, TraceError> {
        let mut frames = Vec::new();
        let mut seen_header = false;

        for (i, raw) in text.lines().enumerate() {
            let line = strip(raw);
            let no = i + 1;
            if line.is_empty() {
                continue;
            }

            if !seen_header {
                if line != HEADER {
                    return err(no, format!("expected header {HEADER:?}, found {line:?}"));
                }
                seen_header = true;
                continue;
            }

            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 3 {
                return err(
                    no,
                    format!(
                        "expected `<dx> <dy> <switches> [x<count>]`, found {} field(s)",
                        fields.len()
                    ),
                );
            }
            if fields.len() > 4 {
                return err(
                    no,
                    format!("unexpected extra field {:?}", fields[4]),
                );
            }

            let dx: i32 = match fields[0].parse() {
                Ok(v) => v,
                Err(_) => return err(no, format!("dx {:?} is not a decimal integer", fields[0])),
            };
            let dy: i32 = match fields[1].parse() {
                Ok(v) => v,
                Err(_) => return err(no, format!("dy {:?} is not a decimal integer", fields[1])),
            };

            let mut switches = Switches::none();
            if fields[2] != "-" {
                for name in fields[2].split(',') {
                    let Some(switch) = switch_from_name(name) else {
                        return err(no, format!("unknown switch {name:?}"));
                    };
                    if switches.contains(switch) {
                        return err(no, format!("switch {name:?} listed twice"));
                    }
                    switches = switches.with(switch);
                }
            }

            let count: usize = match fields.get(3) {
                None => 1,
                Some(field) => {
                    let Some(digits) = field.strip_prefix('x') else {
                        return err(no, format!("expected `x<count>`, found {field:?}"));
                    };
                    match digits.parse() {
                        Ok(0) => return err(no, "repeat count must be at least 1"),
                        Ok(v) => v,
                        Err(_) => return err(no, format!("repeat count {digits:?} is not a number")),
                    }
                }
            };

            frames.extend(std::iter::repeat_n(FrameInput { dx, dy, switches }, count));
        }

        if !seen_header {
            return err(1, format!("empty trace: expected header {HEADER:?}"));
        }
        Ok(Trace { frames })
    }

    /// Write format v1, collapsing runs of identical frames into `x<count>`.
    pub fn serialise(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');

        let mut i = 0;
        while i < self.frames.len() {
            let frame = self.frames[i];
            let mut run = 1;
            while i + run < self.frames.len() && self.frames[i + run] == frame {
                run += 1;
            }
            out.push_str(&frame.to_string());
            if run > 1 {
                out.push_str(&format!(" x{run}"));
            }
            out.push('\n');
            i += run;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Trace {
        Trace {
            frames: vec![
                FrameInput::idle(),
                FrameInput::idle(),
                FrameInput {
                    dx: -7,
                    dy: 13,
                    switches: [Switch::CoinLeft].into_iter().collect(),
                },
                FrameInput {
                    dx: 0,
                    dy: 0,
                    switches: [Switch::Start1, Switch::SelfTest].into_iter().collect(),
                },
                FrameInput::idle(),
            ],
        }
    }

    #[test]
    fn round_trip() {
        let trace = sample();
        let parsed = Trace::parse(&trace.serialise()).expect("round trip");
        assert_eq!(trace, parsed);
    }

    #[test]
    fn runs_collapse_and_expand() {
        let text = Trace::idle(600).serialise();
        assert_eq!(text, format!("{HEADER}\n0 0 - x600\n"));
        assert_eq!(Trace::parse(&text).unwrap().len(), 600);
    }

    #[test]
    fn switches_serialise_in_canonical_order() {
        // Given in reverse; must come back out in ALL_SWITCHES order.
        let switches: Switches = [Switch::CoinRight, Switch::Start1].into_iter().collect();
        assert_eq!(switches.to_string(), "Start1,CoinRight");
        assert_eq!(Switches::none().to_string(), "-");
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let text = "\
# leading comment
\t
chill65-trace 1   # header may carry one too

0 0 -
-1 2 Slam   # a nudge
";
        let trace = Trace::parse(text).expect("parse");
        assert_eq!(trace.len(), 2);
        assert_eq!(trace.frames[1].dx, -1);
        assert_eq!(trace.frames[1].dy, 2);
        assert!(trace.frames[1].switches.contains(Switch::Slam));
    }

    #[test]
    fn negative_and_large_deltas_survive() {
        let text = format!("{HEADER}\n{} {} -\n", i32::MIN, i32::MAX);
        let trace = Trace::parse(&text).expect("parse");
        assert_eq!(trace.frames[0].dx, i32::MIN);
        assert_eq!(trace.frames[0].dy, i32::MAX);
    }

    #[test]
    fn bad_header_is_rejected() {
        let e = Trace::parse("chill65-trace 2\n0 0 -\n").unwrap_err();
        assert_eq!(e.line, 1);
        assert!(e.message.contains("expected header"), "{}", e.message);

        let e = Trace::parse("0 0 -\n").unwrap_err();
        assert_eq!(e.line, 1);

        let e = Trace::parse("# only comments\n").unwrap_err();
        assert!(e.message.contains("empty trace"), "{}", e.message);
    }

    #[test]
    fn unknown_switch_name_is_rejected() {
        let e = Trace::parse(&format!("{HEADER}\n0 0 Start3\n")).unwrap_err();
        assert_eq!(e.line, 2);
        assert!(e.message.contains("unknown switch"), "{}", e.message);

        // Case-sensitive on purpose: the names are the Switch variants.
        let e = Trace::parse(&format!("{HEADER}\n0 0 start1\n")).unwrap_err();
        assert!(e.message.contains("unknown switch"), "{}", e.message);

        let e = Trace::parse(&format!("{HEADER}\n0 0 Slam,Slam\n")).unwrap_err();
        assert!(e.message.contains("twice"), "{}", e.message);
    }

    #[test]
    fn junk_fields_are_rejected() {
        let e = Trace::parse(&format!("{HEADER}\n0 0\n")).unwrap_err();
        assert!(e.message.contains("field(s)"), "{}", e.message);

        let e = Trace::parse(&format!("{HEADER}\n0 0 - x2 rubbish\n")).unwrap_err();
        assert!(e.message.contains("extra field"), "{}", e.message);

        let e = Trace::parse(&format!("{HEADER}\n0 0 - 2\n")).unwrap_err();
        assert!(e.message.contains("x<count>"), "{}", e.message);

        let e = Trace::parse(&format!("{HEADER}\n0 0 - xfoo\n")).unwrap_err();
        assert!(e.message.contains("not a number"), "{}", e.message);

        let e = Trace::parse(&format!("{HEADER}\n0 0 - x0\n")).unwrap_err();
        assert!(e.message.contains("at least 1"), "{}", e.message);

        let e = Trace::parse(&format!("{HEADER}\nfoo 0 -\n")).unwrap_err();
        assert!(e.message.contains("dx"), "{}", e.message);

        let e = Trace::parse(&format!("{HEADER}\n0 bar -\n")).unwrap_err();
        assert!(e.message.contains("dy"), "{}", e.message);
    }

    #[test]
    fn header_only_is_an_empty_trace() {
        let trace = Trace::parse(&format!("{HEADER}\n")).expect("parse");
        assert!(trace.is_empty());
        assert_eq!(trace.serialise(), format!("{HEADER}\n"));
    }
}
