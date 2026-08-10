//! The intermediate representation the Rust emitter lowers from.
//!
//! # Why it is recorded here and not decoded from the image
//!
//! The bytes alone do not say what the source meant. HLL65F's structured
//! control flow — `IFEQ`, `BEGIN`, `ELSE` and the rest — expands to ordinary
//! conditional branches, and once expanded a structured `if` is
//! indistinguishable from a hand-written `BNE`. The structure exists only at
//! macro-expansion time, which is where this records it. `codegen-readiness.md`
//! measured what that is worth: **64.4%** of the game's 876 branches come from
//! HLL65F, and lowering those as native Rust `if`/`loop` is the difference
//! between readable output and a relooper carrying everything.
//!
//! The same argument holds for provenance generally: which unit a routine came
//! from, which macro emitted an instruction, where a label was defined and
//! whether it was private or shared. All of it is free here and unrecoverable
//! later.
//!
//! # It is a recording, not a transformation
//!
//! Recording the IR must not change emission by one byte — the Phase 1 gate
//! asserts the images stay identical to the original toolchain's own output, and
//! that gate does not get weakened for the convenience of a later phase.
//! [`Ir::check_against`] closes the loop by re-encoding every instruction and
//! comparing with the image actually produced.
//!
//! # It is game-derived
//!
//! Run against the corpus, an IR dump describes the game's code and is exactly
//! as undistributable as the ROM (plan §9). Write it under `target/`; never
//! commit it.

use std::collections::BTreeMap;

use crate::encode;
use crate::lexer::Mode;

/// Where a symbol lives: private to its unit, or shared across the build.
///
/// The source declares this itself — `NAME:` and `NAME=` are private, `NAME::`
/// and `NAME==` shared — so this records the rule rather than inventing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Unit,
    Global,
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Scope::Unit => "unit",
            Scope::Global => "global",
        })
    }
}

/// One instruction, as emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Address the first byte landed at.
    pub addr: u16,
    /// Mnemonic as written, upper-cased.
    pub mnemonic: String,
    pub mode: Mode,
    /// Resolved operand, or `None` where the mode carries none.
    pub value: Option<u16>,
    /// The operand as it appeared in the source, for reading the IR back.
    pub operand_text: String,
    /// Encoded length in bytes.
    pub size: u16,
    /// The macro expansion chain at the moment of emission, outermost first.
    /// Empty means the instruction was written directly in the source.
    pub chain: Vec<String>,
    /// The unit being assembled.
    pub unit: String,
    /// The routine this instruction belongs to, filled by
    /// [`Ir::segment_routines`]. `None` means it fell outside any routine —
    /// data areas and the like.
    pub routine: Option<String>,
}

impl Instruction {
    /// Did this instruction come from inside any of `names`?
    pub fn expanded_from(&self, names: &[&str]) -> bool {
        self.chain
            .iter()
            .any(|m| names.contains(&m.to_ascii_uppercase().as_str()))
    }

    /// A conditional branch — the instructions a relooper has to make sense of.
    pub fn is_branch(&self) -> bool {
        matches!(
            self.mnemonic.as_str(),
            "BCC" | "BCS" | "BEQ" | "BMI" | "BNE" | "BPL" | "BVC" | "BVS"
        )
    }
}

/// A structured-control-flow construct, recorded where it was invoked.
///
/// These are HLL65F's, and they are the reason the IR is taken from the
/// assembler at all: after expansion an `IFEQ` is an ordinary `BNE` and nothing
/// distinguishes it from one written by hand. `codegen-readiness.md` measured
/// 64.4% of the game's branches as coming from these constructs, and each one
/// lowered as native Rust `if`/`loop` is one the relooper never has to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker {
    /// `IFEQ`, `IFNE`, … — opens a conditional. The condition is the tested
    /// one, *not* the branch emitted: `DEFIF IFEQ,BNE` means `IFEQ` emits
    /// `BNE`, because the branch jumps over the block (`dialect.md`).
    IfOpen { cond: String },
    /// `ELSE`.
    Else,
    /// `ENDIF`, `THEN`, `ENDC` — closes a conditional.
    IfClose,
    /// `BEGIN` — opens a loop.
    LoopOpen,
    /// `EQEND`, `PLEND`, … — closes a loop, branching back while the condition
    /// holds.
    LoopClose { cond: String },
    /// `EQCONT`, `PLCONT`, `CONTINUE` — jump back to the loop head.
    LoopContinue { cond: String },
}

impl Marker {
    /// Classify a macro name, or `None` if it is not a construct.
    ///
    /// The list is the one `chill65-asm`'s own `analyse()` uses for the O3
    /// measurement, so the two cannot drift apart.
    pub fn classify(name: &str) -> Option<Marker> {
        let up = name.to_ascii_uppercase();
        const CONDS: &[&str] = &["EQ", "NE", "CC", "CS", "MI", "PL", "VC", "VS"];
        match up.as_str() {
            "ELSE" => return Some(Marker::Else),
            "ENDIF" | "THEN" | "ENDC" => return Some(Marker::IfClose),
            "BEGIN" => return Some(Marker::LoopOpen),
            "CONTINUE" => {
                return Some(Marker::LoopContinue {
                    cond: String::from("always"),
                })
            }
            "IF" => {
                return Some(Marker::IfOpen {
                    cond: String::from("always"),
                })
            }
            _ => {}
        }
        for c in CONDS {
            if up == format!("IF{c}") {
                return Some(Marker::IfOpen { cond: c.to_string() });
            }
            if up == format!("{c}END") {
                return Some(Marker::LoopClose { cond: c.to_string() });
            }
            if up == format!("{c}CONT") {
                return Some(Marker::LoopContinue { cond: c.to_string() });
            }
        }
        None
    }

    /// Does this open a block that something must close?
    pub fn opens(&self) -> bool {
        matches!(self, Marker::IfOpen { .. } | Marker::LoopOpen)
    }

    /// Does this close one?
    pub fn closes(&self) -> bool {
        matches!(self, Marker::IfClose | Marker::LoopClose { .. })
    }
}

/// A run of bytes emitted by something other than an instruction — `.BYTE`,
/// `.WORD`, `.BLKB`, a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data {
    pub addr: u16,
    pub len: u16,
    /// The macro chain that produced it, or empty for a direct directive.
    pub chain: Vec<String>,
    pub unit: String,
}

/// A symbol definition, in the order it was defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub addr: u16,
    pub name: String,
    pub scope: Scope,
    pub unit: String,
}

/// One recorded event, in emission order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Instruction(Instruction),
    Data(Data),
    Label(Label),
    /// A structure marker and the address it was invoked at.
    Marker {
        marker: Marker,
        addr: u16,
        unit: String,
    },
}

/// The recorded stream.
#[derive(Debug, Clone, Default)]
pub struct Ir {
    pub events: Vec<Event>,
}

impl Ir {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn instructions(&self) -> impl Iterator<Item = &Instruction> {
        self.events.iter().filter_map(|e| match e {
            Event::Instruction(i) => Some(i),
            _ => None,
        })
    }

    pub fn labels(&self) -> impl Iterator<Item = &Label> {
        self.events.iter().filter_map(|e| match e {
            Event::Label(l) => Some(l),
            _ => None,
        })
    }

    pub fn markers(&self) -> impl Iterator<Item = (&Marker, u16)> {
        self.events.iter().filter_map(|e| match e {
            Event::Marker { marker, addr, .. } => Some((marker, *addr)),
            _ => None,
        })
    }

    pub fn data(&self) -> impl Iterator<Item = &Data> {
        self.events.iter().filter_map(|e| match e {
            Event::Data(d) => Some(d),
            _ => None,
        })
    }

    /// Take every instruction's operand from the resolved image.
    ///
    /// **Structured branches are recorded with a placeholder.** HLL65F's `FND`
    /// emits the branch pointing at itself, then rewinds the location counter
    /// and overwrites the operand byte with a `.BYTE` once the block's length is
    /// known (`dialect.md` §on FND, and its warning: *"Any comparison against
    /// the oracle must therefore use the resolved image, never the record
    /// stream"*). The instruction record is written at emission, so it carries
    /// the placeholder — a self-branch, displacement -2.
    ///
    /// Left alone, an emitter would lower all 564 of the game's structured
    /// branches as infinite self-loops. This reads each operand back out of the
    /// finished image, so the IR carries the target the machine will actually
    /// take, with branch displacements resolved to absolute addresses.
    pub fn resolve_operands_from(&mut self, image: &BTreeMap<u16, u8>) {
        for event in &mut self.events {
            let Event::Instruction(i) = event else {
                continue;
            };
            let at = |k: u16| image.get(&i.addr.wrapping_add(k)).copied();
            match i.size {
                2 => {
                    if let Some(b) = at(1) {
                        i.value = Some(if i.is_branch() {
                            // Relative, from the address after the instruction.
                            i.addr.wrapping_add(2).wrapping_add(b as i8 as u16)
                        } else {
                            b as u16
                        });
                    }
                }
                3 => {
                    if let (Some(lo), Some(hi)) = (at(1), at(2)) {
                        i.value = Some(lo as u16 | ((hi as u16) << 8));
                    }
                }
                // One-byte instructions carry no operand.
                _ => {}
            }
        }
    }

    /// Re-encode every instruction and compare with the image that was
    /// actually produced.
    ///
    /// This checks that each instruction's address, mnemonic, mode and size
    /// describe the bytes really at that address. Operands come *from* the
    /// image via [`Ir::resolve_operands_from`], so they are not independently
    /// verified here — what is verified is that the recorded encoding of each
    /// instruction reproduces the image exactly, which is what the emitter
    /// depends on. Returns one message per disagreement.
    pub fn check_against(&self, image: &BTreeMap<u16, u8>) -> Vec<String> {
        let mut problems = Vec::new();
        for i in self.instructions() {
            let bytes = match encode::encode(&i.mnemonic, i.mode, i.value, i.addr) {
                Ok(b) => b,
                Err(e) => {
                    problems.push(format!("{:04X}: {} does not re-encode: {e}", i.addr, i.mnemonic));
                    continue;
                }
            };
            if bytes.len() as u16 != i.size {
                problems.push(format!(
                    "{:04X}: {} re-encodes to {} bytes, recorded {}",
                    i.addr,
                    i.mnemonic,
                    bytes.len(),
                    i.size
                ));
                continue;
            }
            for (k, b) in bytes.iter().enumerate() {
                let at = i.addr.wrapping_add(k as u16);
                match image.get(&at) {
                    Some(actual) if actual == b => {}
                    Some(actual) => problems.push(format!(
                        "{at:04X}: {} byte {k} is {actual:02X} in the image, IR re-encodes {b:02X}",
                        i.mnemonic
                    )),
                    None => problems.push(format!(
                        "{at:04X}: {} byte {k} is absent from the image",
                        i.mnemonic
                    )),
                }
            }
        }
        problems
    }

    /// Split the program into routines and tag every instruction with the one
    /// it belongs to.
    ///
    /// A routine starts at a label that is either **called or jumped to**, or
    /// **global** — exported, so reachable from another unit whether or not
    /// this unit calls it. Everything from one start up to the next belongs to
    /// it. That is a deliberately structural definition: it needs no heuristics
    /// about what code "looks like" a subroutine, and it is exactly the unit the
    /// emitter generates one Rust function per (plan §3).
    ///
    /// Instructions before the first start, or in data areas, are left `None`.
    pub fn segment_routines(&mut self) {
        // Where control is transferred to.
        let mut targets: std::collections::HashSet<u16> = std::collections::HashSet::new();
        for i in self.instructions() {
            if matches!(i.mnemonic.as_str(), "JSR" | "JMP") {
                if let Some(v) = i.value {
                    targets.insert(v);
                }
            }
        }

        // The first label recorded at each address wins, so the name does not
        // depend on which of several aliases happened to be seen last.
        let mut named: BTreeMap<u16, (String, bool)> = BTreeMap::new();
        for l in self.labels() {
            let global = l.scope == Scope::Global;
            named
                .entry(l.addr)
                .and_modify(|e| e.1 |= global)
                .or_insert((l.name.clone(), global));
        }

        let starts: Vec<(u16, String)> = named
            .into_iter()
            .filter(|(addr, (_, global))| *global || targets.contains(addr))
            .map(|(addr, (name, _))| (addr, name))
            .collect();

        for event in &mut self.events {
            let Event::Instruction(i) = event else {
                continue;
            };
            i.routine = match starts.binary_search_by_key(&i.addr, |(a, _)| *a) {
                Ok(k) => Some(starts[k].1.clone()),
                Err(0) => None,
                Err(k) => Some(starts[k - 1].1.clone()),
            };
        }
    }

    /// Every routine the program was segmented into, in address order.
    pub fn routines(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for i in self.instructions() {
            if let Some(r) = &i.routine {
                if seen.last() != Some(r) {
                    seen.push(r.clone());
                }
            }
        }
        seen.dedup();
        seen
    }

    /// A textual dump, one event per line.
    ///
    /// Game-derived when produced from the corpus: write under `target/`, never
    /// commit.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for event in &self.events {
            match event {
                Event::Instruction(i) => {
                    let chain = if i.chain.is_empty() {
                        String::from("-")
                    } else {
                        i.chain.join(">")
                    };
                    let operand = if i.operand_text.is_empty() {
                        "-"
                    } else {
                        i.operand_text.as_str()
                    };
                    out.push_str(&format!(
                        "{:04X} I {:<5} {:<6} {:<16} size={} unit={} chain={}\n",
                        i.addr,
                        i.mnemonic,
                        format!("{:?}", i.mode),
                        operand,
                        i.size,
                        i.unit,
                        chain
                    ));
                }
                Event::Data(d) => {
                    let chain = if d.chain.is_empty() {
                        String::from("-")
                    } else {
                        d.chain.join(">")
                    };
                    out.push_str(&format!(
                        "{:04X} D len={} unit={} chain={}\n",
                        d.addr, d.len, d.unit, chain
                    ));
                }
                Event::Label(l) => {
                    out.push_str(&format!(
                        "{:04X} L {} [{}] unit={}\n",
                        l.addr, l.name, l.scope, l.unit
                    ));
                }
                Event::Marker { marker, addr, unit } => {
                    out.push_str(&format!("{addr:04X} M {marker:?} unit={unit}\n"));
                }
            }
        }
        out
    }
}
