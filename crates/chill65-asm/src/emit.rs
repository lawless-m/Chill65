//! IR to Rust. Bucket 1 of plan §4: carry HLL65F's structure through.
//!
//! # What a lowered instruction looks like
//!
//! Generated code does **not** reimplement the 6502. It sets `pc` past the
//! opcode and calls the very helper `chill65-runtime`'s interpreter calls:
//!
//! ```text
//! machine.begin_instruction(0xA407);
//! cpu.pc = 0xA408;
//! let c = cpu.load_a(machine, Mode::Immediate, 2);
//! machine.tick(c);
//! ```
//!
//! The helper fetches its own operand, applies the flags, and returns the cycle
//! count *including* the page-crossing penalty. So a compiled instruction and
//! an interpreted one are the same code with the opcode dispatch removed — and
//! the difference a compiled routine can make is confined to control flow,
//! which is the only thing worth compiling.
//!
//! # What is refused
//!
//! Routines whose control flow this pass cannot recover — bare branches with no
//! HLL65F marker, unbalanced constructs, opcodes with no lowering — are
//! **refused by name with a reason** and left interpreted. Never mis-emitted.
//! Plan §5 calls that graceful degradation and it is the reason the creep line
//! works: an unlowerable routine costs performance, not correctness.
//!
//! # Emitted code is game-derived
//!
//! Rust generated from the corpus describes Atari's program and is exactly as
//! undistributable as the ROM (plan §9). Generate it under `target/`; never
//! commit it.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::ir::{Event, Instruction, Ir, Marker};
use crate::lexer::Mode;

/// Which lowering strategy produced a function — plan §4's buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// HLL65F structure carried through as native Rust control flow.
    Structured,
    /// Bare branches, lowered as a block-dispatch state machine.
    ///
    /// Plan §4 ranks this below structure recovery, and for the WASM target it
    /// is: a `loop`/`match` is what LLVM cannot optimise across. For the native
    /// target it is correct and complete, which is what the creep line needs
    /// first. `codegen-readiness.md` measured 266 hand-written branches, 30.4%
    /// of the game's control flow, and the hottest routines are among them.
    StateMachine,
}

/// One generated function.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    /// The routine's entry address, which the registry dispatches on.
    pub entry: u16,
    /// Instructions lowered, for the coverage metric.
    pub instructions: usize,
    pub bucket: Bucket,
}

/// A routine that was not lowered, and why.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub routine: String,
    pub reason: String,
}

/// The result of a lowering run.
#[derive(Debug, Clone, Default)]
pub struct Emitted {
    pub source: String,
    pub functions: Vec<Function>,
    pub refused: Vec<Refusal>,
}

/// The runtime's addressing-mode name for the assembler's.
///
/// The two crates name modes differently — the assembler's are the dialect's
/// notation (`Nx`, `Ay`), the runtime's the processor's — so the mapping is
/// written out rather than assumed.
fn runtime_mode(mode: Mode) -> Option<&'static str> {
    Some(match mode {
        Mode::I => "Immediate",
        Mode::Z => "ZeroPage",
        Mode::Zx => "ZeroPageX",
        Mode::Zy => "ZeroPageY",
        Mode::A => "Absolute",
        Mode::Ax => "AbsoluteX",
        Mode::Ay => "AbsoluteY",
        Mode::N => "Indirect",
        Mode::Nx => "IndirectX",
        Mode::Ny => "IndirectY",
        Mode::Ac => "Accumulator",
        Mode::S => "Relative",
        _ => return None,
    })
}

/// Base cycle count for a mnemonic in a mode, before page-cross and
/// branch-taken penalties — which the runtime helpers add themselves.
fn base_cycles(mnemonic: &str, mode: Mode) -> Option<u8> {
    use Mode::*;
    let c = match (mnemonic, mode) {
        // Loads.
        ("LDA" | "LDX" | "LDY", I) => 2,
        ("LDA" | "LDX" | "LDY", Z) => 3,
        ("LDA" | "LDX" | "LDY", Zx | Zy) => 4,
        ("LDA" | "LDX" | "LDY", A) => 4,
        ("LDA" | "LDX" | "LDY", Ax | Ay) => 4,
        ("LDA", Nx) => 6,
        ("LDA", Ny) => 5,
        // Stores. No immediate form.
        ("STA" | "STX" | "STY", Z) => 3,
        ("STA" | "STX" | "STY", Zx | Zy) => 4,
        ("STA" | "STX" | "STY", A) => 4,
        ("STA", Ax | Ay) => 5,
        ("STA", Nx | Ny) => 6,
        // Bitwise and arithmetic share the load shape.
        ("AND" | "ORA" | "EOR" | "ADC" | "SBC" | "CMP", I) => 2,
        ("AND" | "ORA" | "EOR" | "ADC" | "SBC" | "CMP", Z) => 3,
        ("AND" | "ORA" | "EOR" | "ADC" | "SBC" | "CMP", Zx) => 4,
        ("AND" | "ORA" | "EOR" | "ADC" | "SBC" | "CMP", A | Ax | Ay) => 4,
        ("AND" | "ORA" | "EOR" | "ADC" | "SBC" | "CMP", Nx) => 6,
        ("AND" | "ORA" | "EOR" | "ADC" | "SBC" | "CMP", Ny) => 5,
        ("CPX" | "CPY", I) => 2,
        ("CPX" | "CPY", Z) => 3,
        ("CPX" | "CPY", A) => 4,
        ("BIT", Z) => 3,
        ("BIT", A) => 4,
        // Read-modify-write.
        ("ASL" | "LSR" | "ROL" | "ROR", Ac) => 2,
        ("ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC", Z) => 5,
        ("ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC", Zx) => 6,
        ("ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC", A) => 6,
        ("ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC", Ax) => 7,
        _ => return None,
    };
    Some(c)
}

/// The `Op` variant for a read-modify-write mnemonic.
fn rmw_op(mnemonic: &str) -> Option<&'static str> {
    Some(match mnemonic {
        "INC" => "Inc",
        "DEC" => "Dec",
        "ASL" => "Asl",
        "LSR" => "Lsr",
        "ROL" => "Rol",
        "ROR" => "Ror",
        _ => return None,
    })
}

/// The call that performs one instruction's semantics, or `None` if this pass
/// has no lowering for it.
///
/// Control transfers are handled by the caller, since they end a block rather
/// than merely doing work.
fn semantics(i: &Instruction) -> Option<String> {
    let m = i.mnemonic.as_str();
    let mode = runtime_mode(i.mode)?;
    let c = base_cycles(m, i.mode);

    let call = match m {
        "LDA" => format!("cpu.load_a(machine, Mode::{mode}, {})", c?),
        "LDX" => format!("cpu.load_x(machine, Mode::{mode}, {})", c?),
        "LDY" => format!("cpu.load_y(machine, Mode::{mode}, {})", c?),
        "STA" => format!("cpu.store(machine, Mode::{mode}, cpu.a, {})", c?),
        "STX" => format!("cpu.store(machine, Mode::{mode}, cpu.x, {})", c?),
        "STY" => format!("cpu.store(machine, Mode::{mode}, cpu.y, {})", c?),
        "AND" => format!("cpu.and(machine, Mode::{mode}, {})", c?),
        "ORA" => format!("cpu.ora(machine, Mode::{mode}, {})", c?),
        "EOR" => format!("cpu.eor(machine, Mode::{mode}, {})", c?),
        "BIT" => format!("cpu.bit(machine, Mode::{mode}, {})", c?),
        "ADC" => format!("cpu.adc(machine, Mode::{mode}, {})", c?),
        "SBC" => format!("cpu.sbc(machine, Mode::{mode}, {})", c?),
        "CMP" => format!("cpu.compare(machine, Mode::{mode}, cpu.a, {})", c?),
        "CPX" => format!("cpu.compare(machine, Mode::{mode}, cpu.x, {})", c?),
        "CPY" => format!("cpu.compare(machine, Mode::{mode}, cpu.y, {})", c?),
        "ASL" | "LSR" | "ROL" | "ROR" if i.mode == Mode::Ac => {
            format!("cpu.shift_a(Op::{})", rmw_op(m)?)
        }
        "ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC" => {
            format!("cpu.rmw(machine, Mode::{mode}, Op::{}, {})", rmw_op(m)?, c?)
        }
        // Register and flag operations, which take no operand.
        "TAX" => "cpu.tax()".into(),
        "TAY" => "cpu.tay()".into(),
        "TXA" => "cpu.txa()".into(),
        "TYA" => "cpu.tya()".into(),
        "TSX" => "cpu.tsx()".into(),
        "TXS" => "cpu.txs()".into(),
        "PHA" => "cpu.pha(machine)".into(),
        "PLA" => "cpu.pla(machine)".into(),
        "PHP" => "cpu.php(machine)".into(),
        "PLP" => "cpu.plp(machine)".into(),
        "CLC" => "cpu.set_carry(false)".into(),
        "SEC" => "cpu.set_carry(true)".into(),
        "CLI" => "cpu.set_interrupt_disable(false)".into(),
        "SEI" => "cpu.set_interrupt_disable(true)".into(),
        "CLD" => "cpu.set_decimal(false)".into(),
        "SED" => "cpu.set_decimal(true)".into(),
        "CLV" => "cpu.clv()".into(),
        "INX" => "cpu.inx()".into(),
        "INY" => "cpu.iny()".into(),
        "DEX" => "cpu.dex()".into(),
        "DEY" => "cpu.dey()".into(),
        "NOP" => "2u8".into(),
        _ => return None,
    };
    Some(call)
}

/// The Rust expression testing a branch instruction's own condition.
///
/// Uninverted, unlike a construct's: `BNE` branches when Z is clear.
fn branch_condition(mnemonic: &str) -> Option<&'static str> {
    Some(match mnemonic {
        "BEQ" => "cpu.zero",
        "BNE" => "!cpu.zero",
        "BCS" => "cpu.carry",
        "BCC" => "!cpu.carry",
        "BMI" => "cpu.negative",
        "BPL" => "!cpu.negative",
        "BVS" => "cpu.overflow",
        "BVC" => "!cpu.overflow",
        _ => return None,
    })
}

/// The Rust expression testing an HLL65F condition on the CPU flags.
fn condition(cond: &str) -> Option<&'static str> {
    Some(match cond {
        "EQ" => "cpu.zero",
        "NE" => "!cpu.zero",
        "CS" => "cpu.carry",
        "CC" => "!cpu.carry",
        "MI" => "cpu.negative",
        "PL" => "!cpu.negative",
        "VS" => "cpu.overflow",
        "VC" => "!cpu.overflow",
        _ => return None,
    })
}

/// A routine's events, in emission order.
///
/// Membership is by **address**, with the boundaries decided deliberately.
///
/// A marker is recorded at the location the construct macro was invoked, which
/// is the address of the next instruction to be emitted. So an *opening* marker
/// sits on the first instruction it governs, and a *closing* marker sits on the
/// first instruction after the block. When a block ends a routine, that closer
/// therefore lands on the **next routine's** entry address.
///
/// Left naive, that drops the closer and the routine reads as unbalanced —
/// which is exactly what happened to `MN.ST` and `MN.FRA`, the two hottest
/// routines in the game. So:
///
/// - interior markers belong here;
/// - an opener on the entry address belongs here (a routine opening with
///   `BEGIN`);
/// - a closer on the end address belongs here, not to whatever follows;
/// - and correspondingly a closer on the *entry* address belongs to the
///   previous routine, not this one.
///
/// Each marker thus lands in exactly one routine.
fn routine_events<'a>(ir: &'a Ir, name: &str) -> Vec<&'a Event> {
    let mut lo = u16::MAX;
    let mut hi = 0u16;
    let mut any = false;
    for i in ir.instructions() {
        if i.routine.as_deref() == Some(name) {
            lo = lo.min(i.addr);
            hi = hi.max(i.addr.wrapping_add(i.size));
            any = true;
        }
    }
    if !any {
        return Vec::new();
    }

    ir.events
        .iter()
        .filter(|e| match e {
            Event::Instruction(i) => i.routine.as_deref() == Some(name),
            Event::Marker { marker, addr, .. } => {
                if *addr == lo {
                    !marker.closes()
                } else if *addr == hi {
                    marker.closes()
                } else {
                    *addr > lo && *addr < hi
                }
            }
            _ => false,
        })
        .collect()
}

/// A Rust identifier for a routine name, which may contain `.` and `$`.
fn ident(name: &str) -> String {
    let mut out = String::from("r_");
    for c in name.chars() {
        out.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    out
}

struct Emitter<'a> {
    ir: &'a Ir,
    out: String,
}

impl Emitter<'_> {
    /// Lower one routine, or explain why not.
    fn routine(&mut self, name: &str) -> Result<Function, String> {
        let events = routine_events(self.ir, name);
        if events.is_empty() {
            return Err(format!("no instructions found for {name}"));
        }
        // The entry is the lowest instruction address, not the first event:
        // a routine opening with `BEGIN` has a marker first.
        let entry = events
            .iter()
            .filter_map(|e| match e {
                Event::Instruction(i) => Some(i.addr),
                _ => None,
            })
            .min()
            .ok_or_else(|| format!("{name} has no instructions"))?;

        // Refuse before emitting anything, so a refusal never leaves half a
        // function behind.
        let mut depth = 0i32;
        for e in &events {
            if let Event::Marker { marker, .. } = e {
                if marker.opens() {
                    depth += 1;
                }
                if marker.closes() {
                    depth -= 1;
                }
                if depth < 0 {
                    return Err("a construct closes before it opens".into());
                }
            }
        }
        if depth != 0 {
            return Err(format!("{depth} construct(s) left open"));
        }

        // A routine with any hand-written branch goes through the CFG
        // lowering, markers and all: mixing native structure with a state
        // machine in one function would need the two to agree about where
        // control is, and they have no way to.
        let bare = events.iter().any(|e| match e {
            Event::Instruction(i) => i.is_branch() && i.chain.is_empty(),
            _ => false,
        });

        // Every instruction must have a lowering or a control-flow rule.
        for e in &events {
            if let Event::Instruction(i) = e {
                let known = semantics(i).is_some()
                    || matches!(i.mnemonic.as_str(), "RTS" | "RTI" | "JSR" | "JMP" | "BRK")
                    || (i.is_branch() && branch_condition(&i.mnemonic).is_some());
                if !known {
                    return Err(format!(
                        "no lowering for {} {:?} at {:04X}",
                        i.mnemonic, i.mode, i.addr
                    ));
                }
            }
        }

        let mut body = String::new();
        let mut lowered = 0usize;
        let bucket = if bare {
            self.state_machine(&events, entry, &mut body, &mut lowered)?;
            Bucket::StateMachine
        } else {
            self.block(&events, &mut 0, &mut body, 2, &mut lowered)?;
            Bucket::Structured
        };

        let f = ident(name);
        writeln!(
            self.out,
            "/// `{name}` at `{entry:04X}`, {lowered} instructions.\n\
             pub fn {f}(cpu: &mut Cpu, machine: &mut Machine, deadline: u64) -> u64 {{\n    \
             let mut done = 0u64;\n{body}    done\n}}\n"
        )
        .expect("writing to a String");

        Ok(Function {
            name: name.to_string(),
            entry,
            instructions: lowered,
            bucket,
        })
    }

    /// Lower a routine as a block-dispatch state machine.
    ///
    /// The control-flow graph is recovered from the instructions themselves:
    /// leaders are the entry, every branch target inside the routine, and every
    /// instruction following a transfer. Each block becomes one `match` arm
    /// keyed by its address, and control moves by assigning the next block.
    ///
    /// Using the **address** as the state, rather than an index, is what makes
    /// this safe at the edges: a branch out of the routine, or into the middle
    /// of somewhere with no block of its own, falls to the `_` arm, which sets
    /// `pc` and yields. The interpreter picks up from there. There is no way
    /// for control to end up somewhere this function has an opinion about but
    /// no code for.
    fn state_machine(
        &mut self,
        events: &[&Event],
        entry: u16,
        out: &mut String,
        lowered: &mut usize,
    ) -> Result<(), String> {
        let code: Vec<&Instruction> = events
            .iter()
            .filter_map(|e| match *e {
                Event::Instruction(i) => Some(i),
                _ => None,
            })
            .collect();
        let last = code
            .last()
            .ok_or("state machine over a routine with no instructions")?;
        let end = last.addr.wrapping_add(last.size);
        let inside = |a: u16| a >= entry && a < end;

        // Leaders: the entry, every in-routine branch target, and everything
        // immediately after a transfer.
        let mut leaders: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
        leaders.insert(entry);
        for i in &code {
            let transfers = i.is_branch()
                || matches!(i.mnemonic.as_str(), "JMP" | "JSR" | "RTS" | "RTI" | "BRK");
            if transfers {
                let next = i.addr.wrapping_add(i.size);
                if inside(next) {
                    leaders.insert(next);
                }
                if let Some(t) = i.value {
                    if (i.is_branch() || i.mnemonic == "JMP") && inside(t) {
                        leaders.insert(t);
                    }
                }
            }
        }

        writeln!(out, "    let mut block: u16 = 0x{entry:04X};").unwrap();
        writeln!(out, "    loop {{").unwrap();
        writeln!(out, "        match block {{").unwrap();

        for (n, leader) in leaders.iter().enumerate() {
            let stop = leaders
                .iter()
                .find(|l| *l > leader)
                .copied()
                .unwrap_or(end);
            writeln!(out, "            0x{leader:04X} => {{  // block {n}").unwrap();

            let mut fell_through = true;
            for i in code.iter().filter(|i| i.addr >= *leader && i.addr < stop) {
                *lowered += 1;
                if i.is_branch() {
                    let cond = branch_condition(&i.mnemonic)
                        .ok_or_else(|| format!("no condition for {}", i.mnemonic))?;
                    let next = i.addr.wrapping_add(i.size);
                    let target = i.value.unwrap_or(next);
                    let taken = 3 + u8::from((target & 0xFF00) != (next & 0xFF00));
                    self.yield_check(out, i.addr, 4);
                    writeln!(out, "                // {:04X} {} {}", i.addr, i.mnemonic, i.operand_text).unwrap();
                    writeln!(out, "                machine.begin_instruction(0x{:04X});", i.addr).unwrap();
                    writeln!(out, "                cpu.pc = 0x{next:04X};").unwrap();
                    writeln!(out, "                machine.tick(if {cond} {{ {taken} }} else {{ 2 }});").unwrap();
                    writeln!(out, "                done += 1;").unwrap();
                    // A branch out of the routine yields; one inside moves to
                    // its block.
                    if inside(target) {
                        writeln!(out, "                if {cond} {{ block = 0x{target:04X}; }} else {{ block = 0x{next:04X}; }}").unwrap();
                        writeln!(out, "                continue;").unwrap();
                    } else {
                        writeln!(out, "                if {cond} {{ cpu.pc = 0x{target:04X}; return done; }}").unwrap();
                        writeln!(out, "                block = 0x{next:04X};").unwrap();
                        writeln!(out, "                continue;").unwrap();
                    }
                    fell_through = false;
                    break;
                }
                match i.mnemonic.as_str() {
                    "JMP" if i.mode != Mode::N && i.value.is_some_and(inside) => {
                        let target = i.value.expect("checked");
                        self.yield_check(out, i.addr, 4);
                        writeln!(out, "                // {:04X} JMP {}", i.addr, i.operand_text).unwrap();
                        writeln!(out, "                machine.begin_instruction(0x{:04X});", i.addr).unwrap();
                        writeln!(out, "                machine.tick(3);").unwrap();
                        writeln!(out, "                done += 1;").unwrap();
                        writeln!(out, "                block = 0x{target:04X};").unwrap();
                        writeln!(out, "                continue;").unwrap();
                        fell_through = false;
                        break;
                    }
                    "JMP" | "JSR" | "RTS" | "RTI" | "BRK" => {
                        self.instruction(i, out, 4)?;
                        fell_through = false;
                        break;
                    }
                    _ => self.instruction(i, out, 4)?,
                }
            }

            if fell_through {
                if inside(stop) {
                    writeln!(out, "                block = 0x{stop:04X};").unwrap();
                    writeln!(out, "                continue;").unwrap();
                } else {
                    // Ran off the end of the routine: hand back to the
                    // interpreter rather than guess.
                    writeln!(out, "                cpu.pc = 0x{stop:04X};").unwrap();
                    writeln!(out, "                return done;").unwrap();
                }
            }
            writeln!(out, "            }}").unwrap();
        }

        writeln!(out, "            _ => {{ cpu.pc = block; return done; }}").unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(out, "    }}").unwrap();
        Ok(())
    }

    /// The Compiled contract's yield check, before an instruction.
    fn yield_check(&mut self, out: &mut String, addr: u16, indent: usize) {
        let pad = " ".repeat(indent * 4);
        writeln!(
            out,
            "{pad}if machine.cycles >= deadline || (machine.irq_pending && \
             !cpu.interrupt_disable) {{ cpu.pc = 0x{addr:04X}; return done; }}"
        )
        .unwrap();
    }

    /// Emit events from `at` until this block's closing marker, which is left
    /// for the opener that owns it.
    fn block(
        &mut self,
        events: &[&Event],
        at: &mut usize,
        out: &mut String,
        indent: usize,
        lowered: &mut usize,
    ) -> Result<(), String> {
        let pad = " ".repeat(indent * 4);
        while *at < events.len() {
            match events[*at] {
                // Closers and `else` belong to whoever opened the block. Peek,
                // do not consume: consuming here would end the enclosing block
                // as well.
                Event::Marker { marker, .. }
                    if marker.closes() || matches!(marker, Marker::Else) =>
                {
                    return Ok(());
                }
                Event::Marker { marker, .. } => {
                    *at += 1;
                    match marker {
                        Marker::IfOpen { cond } => {
                            let test = condition(cond)
                                .ok_or_else(|| format!("condition {cond} has no lowering"))?;
                            // HLL65F emits a branch with the *inverse* condition
                            // to skip the block. Its cycles are charged here and
                            // the Rust `if` replaces its jump.
                            // The construct's branch is inverted: taken when
                            // the block is skipped.
                            self.charge_branch(events, at, out, indent, test, false, lowered)?;
                            writeln!(out, "{pad}if {test} {{").unwrap();
                            self.block(events, at, out, indent + 1, lowered)?;

                            if let Some(Event::Marker {
                                marker: Marker::Else,
                                ..
                            }) = events.get(*at).copied()
                            {
                                *at += 1;
                                // `ELSE` emits an unconditional `JMP` over the
                                // else-block (HLL65F.MAC's ELSE: `JMP .`, its
                                // target back-patched by FND). That jump ends
                                // the *if* branch, but it appears after the
                                // marker, so it is charged here — inside the
                                // `if` — and its jump is replaced by the Rust
                                // else.
                                self.charge_jump(events, at, out, indent + 1, lowered)?;
                                writeln!(out, "{pad}}} else {{").unwrap();
                                self.block(events, at, out, indent + 1, lowered)?;
                            }
                            writeln!(out, "{pad}}}").unwrap();

                            match events.get(*at).copied() {
                                Some(Event::Marker {
                                    marker: Marker::IfClose,
                                    ..
                                }) => *at += 1,
                                _ => return Err("conditional is not closed".into()),
                            }
                        }
                        Marker::LoopOpen => {
                            writeln!(out, "{pad}loop {{").unwrap();
                            self.block(events, at, out, indent + 1, lowered)?;

                            // The loop's closing construct names the condition
                            // that ENDS it, so the loop breaks when it holds.
                            let inner = " ".repeat((indent + 1) * 4);
                            match events.get(*at).copied() {
                                Some(Event::Marker {
                                    marker: Marker::LoopClose { cond },
                                    ..
                                }) => {
                                    *at += 1;
                                    let test = condition(cond).ok_or_else(|| {
                                        format!("condition {cond} has no lowering")
                                    })?;
                                    // A loop's closing construct names its
                                    // EXIT condition, and emits the inverse
                                    // branch back to the top: dialect.md
                                    // records `BEGIN … PLEND` assembling to
                                    // `ea 30 fd`, a BMI backwards. So the
                                    // branch is taken while the loop continues
                                    // — when the test does *not* hold.
                                    self.charge_branch(
                                        events,
                                        at,
                                        out,
                                        indent + 1,
                                        test,
                                        false,
                                        lowered,
                                    )?;
                                    writeln!(out, "{inner}if {test} {{ break; }}").unwrap();
                                }
                                _ => return Err("loop is not closed".into()),
                            }
                            writeln!(out, "{pad}}}").unwrap();
                        }
                        Marker::LoopContinue { .. } => {
                            return Err("LoopContinue has no lowering yet".into())
                        }
                        Marker::Else | Marker::IfClose | Marker::LoopClose { .. } => {
                            unreachable!("handled by the peek above")
                        }
                    }
                }
                Event::Instruction(i) => {
                    *at += 1;
                    self.instruction(i, out, indent)?;
                    *lowered += 1;
                }
                _ => {
                    *at += 1;
                }
            }
        }
        Ok(())
    }

    /// Charge a construct's own branch: its cycles, but not its jump.
    ///
    /// Whether the branch is taken when `test` holds depends on the construct.
    /// A conditional's branch is **inverted** — `IFEQ` emits `BNE`, because it
    /// jumps *over* the block — so it is taken when the block does not run. A
    /// loop's closing branch is **direct**: `NEEND` emits `BNE` and takes it to
    /// go round again. Charging both the same way costs a cycle per iteration,
    /// which is exactly the kind of error the differential test exists to
    /// catch.
    fn charge_branch(
        &mut self,
        events: &[&Event],
        at: &mut usize,
        out: &mut String,
        indent: usize,
        test: &str,
        taken_when_true: bool,
        lowered: &mut usize,
    ) -> Result<(), String> {
        let pad = " ".repeat(indent * 4);
        let Some(Event::Instruction(i)) = events.get(*at).copied() else {
            return Err("a construct opened without emitting its branch".into());
        };
        if !i.is_branch() {
            return Err(format!(
                "expected a branch after the construct, found {} at {:04X}",
                i.mnemonic, i.addr
            ));
        }
        *at += 1;
        *lowered += 1;

        let target = i.value.unwrap_or(i.addr.wrapping_add(2));
        let next = i.addr.wrapping_add(2);
        // Two cycles, plus one when taken, plus one more when the taken branch
        // crosses a page — all statically known, so no test is emitted for it.
        let taken = 3 + u8::from((target & 0xFF00) != (next & 0xFF00));

        writeln!(
            out,
            "{pad}// {:04X} {} — the construct's own branch: {taken} taken, 2 not",
            i.addr, i.mnemonic
        )
        .unwrap();
        writeln!(
            out,
            "{pad}if machine.cycles >= deadline || (machine.irq_pending && \
             !cpu.interrupt_disable) {{ cpu.pc = 0x{:04X}; return done; }}",
            i.addr
        )
        .unwrap();
        writeln!(out, "{pad}machine.begin_instruction(0x{:04X});", i.addr).unwrap();
        writeln!(out, "{pad}cpu.pc = 0x{next:04X};").unwrap();
        let (yes, no) = if taken_when_true {
            (taken.to_string(), String::from("2"))
        } else {
            (String::from("2"), taken.to_string())
        };
        writeln!(out, "{pad}machine.tick(if {test} {{ {yes} }} else {{ {no} }});").unwrap();
        writeln!(out, "{pad}done += 1;").unwrap();
        Ok(())
    }

    /// Charge an unconditional jump's cycles without emitting its jump.
    fn charge_jump(
        &mut self,
        events: &[&Event],
        at: &mut usize,
        out: &mut String,
        indent: usize,
        lowered: &mut usize,
    ) -> Result<(), String> {
        let pad = " ".repeat(indent * 4);
        let Some(Event::Instruction(i)) = events.get(*at).copied() else {
            return Err("ELSE emitted no jump".into());
        };
        if i.mnemonic != "JMP" {
            return Err(format!(
                "expected ELSE's jump, found {} at {:04X}",
                i.mnemonic, i.addr
            ));
        }
        *at += 1;
        *lowered += 1;
        writeln!(out, "{pad}// {:04X} JMP — ELSE's jump over the else-block", i.addr).unwrap();
        writeln!(
            out,
            "{pad}if machine.cycles >= deadline || (machine.irq_pending && \
             !cpu.interrupt_disable) {{ cpu.pc = 0x{:04X}; return done; }}",
            i.addr
        )
        .unwrap();
        writeln!(out, "{pad}machine.begin_instruction(0x{:04X});", i.addr).unwrap();
        writeln!(out, "{pad}machine.tick(3);").unwrap();
        writeln!(out, "{pad}done += 1;").unwrap();
        Ok(())
    }

    /// One instruction: the yield check, then its semantics.
    fn instruction(
        &mut self,
        i: &Instruction,
        out: &mut String,
        indent: usize,
    ) -> Result<(), String> {
        let pad = " ".repeat(indent * 4);
        let next = i.addr.wrapping_add(i.size);

        // The Compiled contract: check exactly what the interpreter's loop
        // checks, and leave pc on the next unexecuted instruction.
        writeln!(
            out,
            "{pad}if machine.cycles >= deadline || (machine.irq_pending && !cpu.interrupt_disable) \
             {{ cpu.pc = 0x{:04X}; return done; }}",
            i.addr
        )
        .unwrap();

        match i.mnemonic.as_str() {
            "RTS" | "RTI" | "BRK" | "JSR" | "JMP" => {
                let call = match i.mnemonic.as_str() {
                    "RTS" => "cpu.rts(machine)",
                    "RTI" => "cpu.rti(machine)",
                    "BRK" => "cpu.brk(machine)",
                    "JSR" => "cpu.jsr(machine)",
                    "JMP" if i.mode == Mode::N => "cpu.jmp_indirect(machine)",
                    _ => "cpu.jmp_absolute(machine)",
                };
                writeln!(out, "{pad}// {:04X} {}", i.addr, i.mnemonic).unwrap();
                writeln!(out, "{pad}machine.begin_instruction(0x{:04X});", i.addr).unwrap();
                writeln!(out, "{pad}cpu.pc = 0x{:04X};", i.addr.wrapping_add(1)).unwrap();
                writeln!(out, "{pad}let c = {call};").unwrap();
                writeln!(out, "{pad}machine.tick(c);").unwrap();
                writeln!(out, "{pad}done += 1;").unwrap();
                // Control left the routine: the dispatch gets another chance at
                // wherever pc now points.
                writeln!(out, "{pad}return done;").unwrap();
            }
            _ => {
                let call = semantics(i)
                    .ok_or_else(|| format!("no lowering for {} at {:04X}", i.mnemonic, i.addr))?;
                writeln!(
                    out,
                    "{pad}// {:04X} {} {}",
                    i.addr, i.mnemonic, i.operand_text
                )
                .unwrap();
                writeln!(out, "{pad}machine.begin_instruction(0x{:04X});", i.addr).unwrap();
                writeln!(out, "{pad}cpu.pc = 0x{:04X};", i.addr.wrapping_add(1)).unwrap();
                writeln!(out, "{pad}let c = {call};").unwrap();
                writeln!(out, "{pad}machine.tick(c);").unwrap();
                writeln!(out, "{pad}cpu.pc = 0x{next:04X};").unwrap();
                writeln!(out, "{pad}done += 1;").unwrap();
            }
        }
        Ok(())
    }
}

/// Lower the named routines, refusing the ones this pass cannot handle.
pub fn emit(ir: &Ir, want: &[&str]) -> Emitted {
    let mut e = Emitter {
        ir,
        out: String::new(),
    };
    writeln!(
        e.out,
        "// Generated by chill65-asm. Do not edit, and do not commit: emitted\n\
         // from the game source, this is as game-derived as the ROM (plan §9).\n\
         // Included into a module, so no inner attributes here.\n\
         use chill65_runtime::addr::Mode;\n\
         use chill65_runtime::exec::Op;\n\
         use chill65_runtime::{{Cpu, Machine}};\n\
         use chill65_runtime::bus::Bus;\n"
    )
    .unwrap();

    let mut functions = Vec::new();
    let mut refused = Vec::new();
    let all = ir.routines();
    let known: BTreeMap<&str, ()> = all.iter().map(|r| (r.as_str(), ())).collect();

    for name in want {
        if !known.contains_key(*name) {
            refused.push(Refusal {
                routine: (*name).to_string(),
                reason: "no such routine in the IR".into(),
            });
            continue;
        }
        let before = e.out.len();
        match e.routine(name) {
            Ok(f) => functions.push(f),
            Err(reason) => {
                e.out.truncate(before);
                refused.push(Refusal {
                    routine: (*name).to_string(),
                    reason,
                });
            }
        }
    }

    // The registry the dispatch consults, entry address to function.
    writeln!(
        e.out,
        "/// Entry address to generated function, for the `Compiled` dispatch.\n\
         pub const ROUTINES: &[(u16, fn(&mut Cpu, &mut Machine, u64) -> u64)] = &["
    )
    .unwrap();
    for f in &functions {
        writeln!(e.out, "    (0x{:04X}, {}),", f.entry, ident(&f.name)).unwrap();
    }
    writeln!(e.out, "];\n").unwrap();

    writeln!(
        e.out,
        "/// Per-routine metadata, for the coverage report.\n\
         pub const METADATA: &[(&str, u16, usize, &str)] = &["
    )
    .unwrap();
    for f in &functions {
        writeln!(
            e.out,
            "    ({:?}, 0x{:04X}, {}, {:?}),",
            f.name,
            f.entry,
            f.instructions,
            format!("{:?}", f.bucket)
        )
        .unwrap();
    }
    writeln!(e.out, "];").unwrap();

    Emitted {
        source: e.out,
        functions,
        refused,
    }
}
