//! 6502 instruction encoding, in the Atari dialect's prefix syntax.
//!
//! # Where the tables come from
//!
//! Atari's own permanent symbol table,
//! `atari-coin-op-assembler/atari_tools/e3_tools/PST65.MAC`, defines every
//! opcode as
//!
//! ```text
//! OPCDEF	NAME,CLASS,VALUE,AM,FLAGS
//! OPCDEF	ADC,02,141,MOST
//! MOST=I+A+Z+NX+NY+ZX+AX+AY
//! ```
//!
//! with the mode bits assigned in order by
//! `.IRP X,<NX,Z,I,A,NY,ZX,AY,AX,R,AC,N,ZY>`. `VALUE` is octal, and for the
//! regular classes the encoding is `base + 4 × slot`, which is exactly what
//! `OPC65.MAC`'s operand processor does ("UPDATE OPCODE TO CORRECT ADDRESS
//! MODE"). `AC` folds onto `I`'s slot — `OPC65.MAC` says so outright, forcing
//! `AM.I` when an accumulator-mode instruction appears with no operand. `SPEC`
//! marks the instructions that break the pattern: `CPX`, `CPY`, `LDX`, `LDY`,
//! `STX`, `STY`, `JMP`.
//!
//! Rather than reimplement that bit arithmetic, the table below is explicit and
//! checkable against any published 6502 reference. PST65 is used for the part
//! that would otherwise be guesswork: **which modes each mnemonic accepts**.
//! A mnemonic that is not in this table is a macro, not an instruction — the
//! corpus supplies its instruction set through `M6502.MAC`, so inventing
//! encodings would be actively harmful.
//!
//! Note `PST65.MAC` calls the relative mode `R` where `OPC65.MAC`'s `AMCHR`
//! calls it `S`; they are the same mode. [`Mode::S`] is used throughout.

use crate::lexer::Mode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    UnknownMnemonic(String),
    IllegalMode { mnemonic: String, mode: Mode },
    BranchOutOfRange { mnemonic: String, offset: i32 },
    OperandRequired(String),
    OperandTooLarge { mnemonic: String, value: u16 },
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::UnknownMnemonic(m) => write!(f, "{m} is not a 6502 instruction"),
            EncodeError::IllegalMode { mnemonic, mode } => {
                write!(f, "{mnemonic} does not accept addressing mode {mode:?}")
            }
            EncodeError::BranchOutOfRange { mnemonic, offset } => write!(
                f,
                "{mnemonic} branch out of range: offset {offset} is outside -128..127"
            ),
            EncodeError::OperandRequired(m) => write!(f, "{m} requires an operand"),
            EncodeError::OperandTooLarge { mnemonic, value } => {
                write!(f, "{mnemonic} zero-page operand {value:#x} exceeds 0xFF")
            }
        }
    }
}

/// `(mnemonic, &[(mode, opcode)])`, transcribed against the published 6502
/// matrix; the legal-mode sets match `PST65.MAC`'s `AM` field entry for entry.
#[rustfmt::skip]
const TABLE: &[(&str, &[(Mode, u8)])] = &[
    // Implied / no operand — PST65 class 00.
    ("BRK", &[(Mode::Ac, 0x00)]), ("CLC", &[(Mode::Ac, 0x18)]),
    ("CLD", &[(Mode::Ac, 0xD8)]), ("CLI", &[(Mode::Ac, 0x58)]),
    ("CLV", &[(Mode::Ac, 0xB8)]), ("DEX", &[(Mode::Ac, 0xCA)]),
    ("DEY", &[(Mode::Ac, 0x88)]), ("INX", &[(Mode::Ac, 0xE8)]),
    ("INY", &[(Mode::Ac, 0xC8)]), ("NOP", &[(Mode::Ac, 0xEA)]),
    ("PHA", &[(Mode::Ac, 0x48)]), ("PHP", &[(Mode::Ac, 0x08)]),
    ("PLA", &[(Mode::Ac, 0x68)]), ("PLP", &[(Mode::Ac, 0x28)]),
    ("RTI", &[(Mode::Ac, 0x40)]), ("RTS", &[(Mode::Ac, 0x60)]),
    ("SEC", &[(Mode::Ac, 0x38)]), ("SED", &[(Mode::Ac, 0xF8)]),
    ("SEI", &[(Mode::Ac, 0x78)]), ("TAX", &[(Mode::Ac, 0xAA)]),
    ("TAY", &[(Mode::Ac, 0xA8)]), ("TSX", &[(Mode::Ac, 0xBA)]),
    ("TXA", &[(Mode::Ac, 0x8A)]), ("TXS", &[(Mode::Ac, 0x9A)]),
    ("TYA", &[(Mode::Ac, 0x98)]),

    // Branches — PST65 class 03, mode R (Mode::S here).
    ("BCC", &[(Mode::S, 0x90)]), ("BCS", &[(Mode::S, 0xB0)]),
    ("BEQ", &[(Mode::S, 0xF0)]), ("BMI", &[(Mode::S, 0x30)]),
    ("BNE", &[(Mode::S, 0xD0)]), ("BPL", &[(Mode::S, 0x10)]),
    ("BVC", &[(Mode::S, 0x50)]), ("BVS", &[(Mode::S, 0x70)]),

    // MOST = I+A+Z+NX+NY+ZX+AX+AY
    ("ADC", &[(Mode::Nx,0x61),(Mode::Z,0x65),(Mode::I,0x69),(Mode::A,0x6D),
              (Mode::Ny,0x71),(Mode::Zx,0x75),(Mode::Ay,0x79),(Mode::Ax,0x7D)]),
    ("AND", &[(Mode::Nx,0x21),(Mode::Z,0x25),(Mode::I,0x29),(Mode::A,0x2D),
              (Mode::Ny,0x31),(Mode::Zx,0x35),(Mode::Ay,0x39),(Mode::Ax,0x3D)]),
    ("CMP", &[(Mode::Nx,0xC1),(Mode::Z,0xC5),(Mode::I,0xC9),(Mode::A,0xCD),
              (Mode::Ny,0xD1),(Mode::Zx,0xD5),(Mode::Ay,0xD9),(Mode::Ax,0xDD)]),
    ("EOR", &[(Mode::Nx,0x41),(Mode::Z,0x45),(Mode::I,0x49),(Mode::A,0x4D),
              (Mode::Ny,0x51),(Mode::Zx,0x55),(Mode::Ay,0x59),(Mode::Ax,0x5D)]),
    ("LDA", &[(Mode::Nx,0xA1),(Mode::Z,0xA5),(Mode::I,0xA9),(Mode::A,0xAD),
              (Mode::Ny,0xB1),(Mode::Zx,0xB5),(Mode::Ay,0xB9),(Mode::Ax,0xBD)]),
    ("ORA", &[(Mode::Nx,0x01),(Mode::Z,0x05),(Mode::I,0x09),(Mode::A,0x0D),
              (Mode::Ny,0x11),(Mode::Zx,0x15),(Mode::Ay,0x19),(Mode::Ax,0x1D)]),
    ("SBC", &[(Mode::Nx,0xE1),(Mode::Z,0xE5),(Mode::I,0xE9),(Mode::A,0xED),
              (Mode::Ny,0xF1),(Mode::Zx,0xF5),(Mode::Ay,0xF9),(Mode::Ax,0xFD)]),
    // STA is MOST-I — no immediate form, as PST65 spells out.
    ("STA", &[(Mode::Nx,0x81),(Mode::Z,0x85),(Mode::A,0x8D),
              (Mode::Ny,0x91),(Mode::Zx,0x95),(Mode::Ay,0x99),(Mode::Ax,0x9D)]),

    // Read-modify-write: A+Z+AC+ZX+AX
    ("ASL", &[(Mode::Z,0x06),(Mode::Ac,0x0A),(Mode::A,0x0E),(Mode::Zx,0x16),(Mode::Ax,0x1E)]),
    ("LSR", &[(Mode::Z,0x46),(Mode::Ac,0x4A),(Mode::A,0x4E),(Mode::Zx,0x56),(Mode::Ax,0x5E)]),
    ("ROL", &[(Mode::Z,0x26),(Mode::Ac,0x2A),(Mode::A,0x2E),(Mode::Zx,0x36),(Mode::Ax,0x3E)]),
    ("ROR", &[(Mode::Z,0x66),(Mode::Ac,0x6A),(Mode::A,0x6E),(Mode::Zx,0x76),(Mode::Ax,0x7E)]),
    // A+Z+ZX+AX, no accumulator form
    ("DEC", &[(Mode::Z,0xC6),(Mode::A,0xCE),(Mode::Zx,0xD6),(Mode::Ax,0xDE)]),
    ("INC", &[(Mode::Z,0xE6),(Mode::A,0xEE),(Mode::Zx,0xF6),(Mode::Ax,0xFE)]),

    // SPEC group — the ones whose mode-to-opcode mapping breaks the pattern.
    ("BIT", &[(Mode::Z,0x24),(Mode::A,0x2C)]),
    ("CPX", &[(Mode::I,0xE0),(Mode::Z,0xE4),(Mode::A,0xEC)]),
    ("CPY", &[(Mode::I,0xC0),(Mode::Z,0xC4),(Mode::A,0xCC)]),
    ("LDX", &[(Mode::I,0xA2),(Mode::Z,0xA6),(Mode::A,0xAE),(Mode::Zy,0xB6),(Mode::Ay,0xBE)]),
    ("LDY", &[(Mode::I,0xA0),(Mode::Z,0xA4),(Mode::A,0xAC),(Mode::Zx,0xB4),(Mode::Ax,0xBC)]),
    ("STX", &[(Mode::Z,0x86),(Mode::A,0x8E),(Mode::Zy,0x96)]),
    ("STY", &[(Mode::Z,0x84),(Mode::A,0x8C),(Mode::Zx,0x94)]),
    ("JMP", &[(Mode::A,0x4C),(Mode::N,0x6C)]),
    ("JSR", &[(Mode::A,0x20)]),
];

fn lookup(mnemonic: &str) -> Option<&'static [(Mode, u8)]> {
    let m = mnemonic.to_ascii_uppercase();
    TABLE.iter().find(|(n, _)| *n == m).map(|(_, t)| *t)
}

/// Is this a 6502 instruction at all? Anything else on a line is a macro call.
pub fn is_instruction(mnemonic: &str) -> bool {
    lookup(mnemonic).is_some()
}

fn opcode_for(mnemonic: &str, mode: Mode) -> Option<u8> {
    lookup(mnemonic)?
        .iter()
        .find(|(m, _)| *m == mode)
        .map(|(_, op)| *op)
}

fn accepts(mnemonic: &str, mode: Mode) -> bool {
    opcode_for(mnemonic, mode).is_some()
}

/// Resolve the auto-sizing and default modes to a concrete one.
///
/// `X` and `Y` are the forms used pervasively in the corpus (`LDA X,VITAB`),
/// and a bare operand with no prefix is likewise unsized. With `.ENABL AMA`
/// ("auto zero page management") an operand below `0x100` takes the zero-page
/// form where one exists, which makes the instruction two bytes instead of
/// three — and therefore moves every address after it.
pub fn resolve_mode(
    mnemonic: &str,
    written: Option<Mode>,
    has_operand: bool,
    operand: Option<u16>,
    ama: bool,
    byte_sized: bool,
) -> Result<Mode, EncodeError> {
    // Two independent routes to a short operand. AMA sizes from the *value*, so
    // it needs one; `.GLOBB` sizes from the *declaration*, which is what lets an
    // import whose value this unit cannot see still take a zero-page operand.
    let zp_ok = |v: Option<u16>| byte_sized || (ama && v.is_some_and(|v| v < 0x100));

    let mode = match written {
        // Explicitly written modes pass straight through.
        Some(m) if !matches!(m, Mode::X | Mode::Y) => m,

        Some(Mode::X) => {
            if zp_ok(operand) && accepts(mnemonic, Mode::Zx) {
                Mode::Zx
            } else {
                Mode::Ax
            }
        }
        Some(Mode::Y) => {
            if zp_ok(operand) && accepts(mnemonic, Mode::Zy) {
                Mode::Zy
            } else if accepts(mnemonic, Mode::Ay) {
                Mode::Ay
            } else {
                Mode::Zy
            }
        }

        // No prefix at all.
        //
        // `has_operand` is distinct from `operand`: in pass one a forward
        // reference is written but not yet resolvable, so its value is None
        // while an operand plainly exists. Conflating the two makes
        // `BNE FORWARD` look like an operandless instruction.
        None => {
            if !has_operand {
                // No operand: accumulator/implied if legal.
                if accepts(mnemonic, Mode::Ac) {
                    Mode::Ac
                } else {
                    return Err(EncodeError::OperandRequired(mnemonic.to_string()));
                }
            } else if accepts(mnemonic, Mode::S) {
                // Branches take a target address, not a mode.
                Mode::S
            } else if zp_ok(operand) && accepts(mnemonic, Mode::Z) {
                Mode::Z
            } else {
                Mode::A
            }
        }
        Some(m) => m,
    };

    if !accepts(mnemonic, mode) {
        return Err(EncodeError::IllegalMode {
            mnemonic: mnemonic.to_string(),
            mode,
        });
    }
    Ok(mode)
}

/// How many bytes an instruction occupies in a given mode.
pub fn size_of(mode: Mode) -> u16 {
    match mode {
        Mode::Ac => 1,
        Mode::I | Mode::Z | Mode::Zx | Mode::Zy | Mode::Nx | Mode::Ny | Mode::S => 2,
        Mode::A | Mode::Ax | Mode::Ay | Mode::N => 3,
        // Unresolved auto-sizing forms should never reach here.
        Mode::X | Mode::Y => 3,
    }
}

/// Encode one instruction.
///
/// `loc` is the address of the opcode byte, needed for relative branches.
pub fn encode(
    mnemonic: &str,
    mode: Mode,
    operand: Option<u16>,
    loc: u16,
) -> Result<Vec<u8>, EncodeError> {
    let op = opcode_for(mnemonic, mode).ok_or_else(|| {
        if lookup(mnemonic).is_none() {
            EncodeError::UnknownMnemonic(mnemonic.to_string())
        } else {
            EncodeError::IllegalMode {
                mnemonic: mnemonic.to_string(),
                mode,
            }
        }
    })?;

    match mode {
        Mode::Ac => Ok(vec![op]),

        Mode::S => {
            let target = operand.ok_or_else(|| EncodeError::OperandRequired(mnemonic.into()))?;
            // Relative to the byte after the two-byte instruction — the same
            // arithmetic HLL65F's FND performs: ...S0 = ...P0-...P1-2
            let delta = target.wrapping_sub(loc.wrapping_add(2)) as i16 as i32;
            if !(-128..=127).contains(&delta) {
                return Err(EncodeError::BranchOutOfRange {
                    mnemonic: mnemonic.to_string(),
                    offset: delta,
                });
            }
            Ok(vec![op, delta as u8])
        }

        Mode::I | Mode::Z | Mode::Zx | Mode::Zy | Mode::Nx | Mode::Ny => {
            let v = operand.ok_or_else(|| EncodeError::OperandRequired(mnemonic.into()))?;
            if !matches!(mode, Mode::I) && v > 0xFF {
                return Err(EncodeError::OperandTooLarge {
                    mnemonic: mnemonic.to_string(),
                    value: v,
                });
            }
            Ok(vec![op, v as u8])
        }

        Mode::A | Mode::Ax | Mode::Ay | Mode::N => {
            let v = operand.ok_or_else(|| EncodeError::OperandRequired(mnemonic.into()))?;
            Ok(vec![op, v as u8, (v >> 8) as u8])
        }

        Mode::X | Mode::Y => Err(EncodeError::IllegalMode {
            mnemonic: mnemonic.to_string(),
            mode,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(m: &str, mode: Mode, operand: Option<u16>) -> Vec<u8> {
        encode(m, mode, operand, 0).unwrap_or_else(|e| panic!("{m} {mode:?}: {e}"))
    }

    #[test]
    fn every_prefix_encodes_to_the_right_opcode_and_length() {
        // One representative per addressing mode, against the published matrix.
        assert_eq!(enc("LDA", Mode::Nx, Some(0x20)), vec![0xA1, 0x20]);
        assert_eq!(enc("LDA", Mode::Z, Some(0x20)), vec![0xA5, 0x20]);
        assert_eq!(enc("LDA", Mode::I, Some(0x20)), vec![0xA9, 0x20]);
        assert_eq!(enc("LDA", Mode::A, Some(0x1234)), vec![0xAD, 0x34, 0x12]);
        assert_eq!(enc("LDA", Mode::Ny, Some(0x20)), vec![0xB1, 0x20]);
        assert_eq!(enc("LDA", Mode::Zx, Some(0x20)), vec![0xB5, 0x20]);
        assert_eq!(enc("LDA", Mode::Ay, Some(0x1234)), vec![0xB9, 0x34, 0x12]);
        assert_eq!(enc("LDA", Mode::Ax, Some(0x1234)), vec![0xBD, 0x34, 0x12]);
        assert_eq!(enc("LDX", Mode::Zy, Some(0x20)), vec![0xB6, 0x20]);
        assert_eq!(enc("JMP", Mode::N, Some(0x1234)), vec![0x6C, 0x34, 0x12]);
        assert_eq!(enc("ASL", Mode::Ac, None), vec![0x0A]);
        assert_eq!(enc("BNE", Mode::S, Some(0x0002)), vec![0xD0, 0x00]);

        // Lengths follow from the mode alone.
        assert_eq!(size_of(Mode::Ac), 1);
        assert_eq!(size_of(Mode::Z), 2);
        assert_eq!(size_of(Mode::A), 3);
    }

    #[test]
    fn ama_decides_zero_page_versus_absolute() {
        // With AMA on, an operand below 0x100 takes the zero-page form.
        let m = resolve_mode("LDA", None, true, Some(0x04), true, false).unwrap();
        assert_eq!(m, Mode::Z);
        assert_eq!(size_of(m), 2);
        assert_eq!(enc("LDA", m, Some(0x04)), vec![0xA5, 0x04]);

        // With AMA off it stays absolute — three bytes, moving everything after.
        let m = resolve_mode("LDA", None, true, Some(0x04), false, false).unwrap();
        assert_eq!(m, Mode::A);
        assert_eq!(size_of(m), 3);
        assert_eq!(enc("LDA", m, Some(0x04)), vec![0xAD, 0x04, 0x00]);

        // Above the zero page, AMA makes no difference.
        assert_eq!(resolve_mode("LDA", None, true, Some(0x9E87), true, false).unwrap(), Mode::A);
    }

    #[test]
    fn unresolved_forward_reference_is_still_an_operand() {
        // Pass one cannot resolve a forward-referenced branch target, but the
        // operand is plainly there — treating None as "no operand" made
        // `BNE FORWARD` look operandless and picked an illegal mode.
        assert_eq!(
            resolve_mode("BNE", None, true, None, false, false).unwrap(),
            Mode::S
        );
        assert_eq!(resolve_mode("LDA", None, true, None, true, false).unwrap(), Mode::A);
        // Genuinely operandless still works.
        assert_eq!(resolve_mode("RTS", None, false, None, false, false).unwrap(), Mode::Ac);
        assert!(resolve_mode("LDA", None, false, None, false, false).is_err());
    }

    #[test]
    fn auto_sizing_x_and_y_prefixes() {
        // LDA X,VITAB — zero page when AMA allows, absolute otherwise.
        assert_eq!(resolve_mode("LDA", Some(Mode::X), true, Some(0x30), true, false).unwrap(), Mode::Zx);
        assert_eq!(resolve_mode("LDA", Some(Mode::X), true, Some(0x8030), true, false).unwrap(), Mode::Ax);
        assert_eq!(resolve_mode("LDA", Some(Mode::X), true, Some(0x30), false, false).unwrap(), Mode::Ax);
        // LDX has no zp,X — only zp,Y — so Y auto-sizes to Zy.
        assert_eq!(resolve_mode("LDX", Some(Mode::Y), true, Some(0x30), true, false).unwrap(), Mode::Zy);
        // LDA has no zp,Y, so Y must land on absolute,Y.
        assert_eq!(resolve_mode("LDA", Some(Mode::Y), true, Some(0x30), true, false).unwrap(), Mode::Ay);
    }

    #[test]
    fn branch_range_is_checked_not_wrapped() {
        // In range: +127 from the end of the instruction.
        assert!(encode("BNE", Mode::S, Some(0x0081), 0x0000).is_ok());
        // 130 bytes away — must error rather than silently wrap.
        let e = encode("BNE", Mode::S, Some(130), 0x0000).unwrap_err();
        assert!(
            matches!(e, EncodeError::BranchOutOfRange { .. }),
            "expected out-of-range, got {e}"
        );
        // Backwards works too.
        assert_eq!(encode("BPL", Mode::S, Some(0x1000), 0x1010).unwrap(), vec![0x10, 0xEE]);
        let e = encode("BPL", Mode::S, Some(0x1000), 0x1090).unwrap_err();
        assert!(matches!(e, EncodeError::BranchOutOfRange { .. }));
    }

    #[test]
    fn illegal_modes_are_rejected_per_pst65() {
        // PST65 gives STA the mode set MOST-I: no immediate form.
        assert!(matches!(
            encode("STA", Mode::I, Some(1), 0),
            Err(EncodeError::IllegalMode { .. })
        ));
        // BIT is A+Z only.
        assert!(matches!(
            encode("BIT", Mode::I, Some(1), 0),
            Err(EncodeError::IllegalMode { .. })
        ));
        // INC has no accumulator form; ASL does.
        assert!(matches!(
            encode("INC", Mode::Ac, None, 0),
            Err(EncodeError::IllegalMode { .. })
        ));
        assert!(encode("ASL", Mode::Ac, None, 0).is_ok());
    }

    #[test]
    fn macros_are_not_mistaken_for_instructions() {
        // The corpus supplies its instruction set through M6502.MAC; these are
        // macros, and inventing encodings for them would be actively harmful.
        for m in ["TRAI", "TRAM", "TR16AI", "ADAI", "INXS", "BEGIN", "IFEQ", "PLEND"] {
            assert!(!is_instruction(m), "{m} must not be treated as an opcode");
        }
        for m in ["LDA", "STA", "JMP", "BNE", "RTS"] {
            assert!(is_instruction(m), "{m} is a real instruction");
        }
        assert!(matches!(
            encode("TRAI", Mode::I, Some(0), 0),
            Err(EncodeError::UnknownMnemonic(_))
        ));
    }

    /// Round trip against real ROM bytes.
    ///
    /// The reset vector points at `E000`, and the extracted `CRF.LDA` image
    /// holds `a9 00 8d 87 9e 4c 06 e1` there. That is `CIN.MAC:7`
    /// (`TRAI 0 HW.BSL  ; select bank 0 and jump`) expanded through `M6502.MAC`,
    /// plus the jump that follows — with `HW.BSL = 9E87` from `CG.MAC`.
    #[test]
    fn round_trip_against_real_rom_bytes() {
        let mut out = Vec::new();
        // TRAI 0 HW.BSL  ->  LDA I,0 / STA HW.BSL
        out.extend(encode("LDA", Mode::I, Some(0x00), 0xE000).unwrap());
        out.extend(encode("STA", Mode::A, Some(0x9E87), 0xE002).unwrap());
        // JMP E106
        out.extend(encode("JMP", Mode::A, Some(0xE106), 0xE005).unwrap());

        assert_eq!(
            out,
            vec![0xA9, 0x00, 0x8D, 0x87, 0x9E, 0x4C, 0x06, 0xE1],
            "does not match the bytes at E000 in the real ROM image"
        );
    }

    #[test]
    fn interrupt_prologue_round_trip() {
        // CIN.MAC's ISR prologue at E009: PHA / TXA / PHA, seen in the image as
        // 48 8a 48.
        let mut out = Vec::new();
        out.extend(encode("PHA", Mode::Ac, None, 0xE009).unwrap());
        out.extend(encode("TXA", Mode::Ac, None, 0xE00A).unwrap());
        out.extend(encode("PHA", Mode::Ac, None, 0xE00B).unwrap());
        assert_eq!(out, vec![0x48, 0x8A, 0x48]);
    }

    #[test]
    fn table_matches_pst65_legal_mode_counts() {
        // PST65's AM field spells out how many modes each mnemonic accepts;
        // guard the transcription against silent drift.
        let counts = [
            ("LDA", 8), ("ADC", 8), ("AND", 8), ("CMP", 8), ("EOR", 8),
            ("ORA", 8), ("SBC", 8),          // MOST
            ("STA", 7),                       // MOST-I
            ("ASL", 5), ("LSR", 5), ("ROL", 5), ("ROR", 5),
            ("DEC", 4), ("INC", 4),
            ("CPX", 3), ("CPY", 3), ("BIT", 2),
            ("LDX", 5), ("LDY", 5), ("STX", 3), ("STY", 3),
            ("JMP", 2), ("JSR", 1),
            ("BNE", 1), ("RTS", 1),
        ];
        for (m, n) in counts {
            assert_eq!(lookup(m).unwrap().len(), n, "{m} mode count");
        }
        // And no mnemonic maps two modes to the same opcode.
        for (name, modes) in TABLE {
            let mut seen: Vec<u8> = modes.iter().map(|(_, o)| *o).collect();
            seen.sort_unstable();
            let before = seen.len();
            seen.dedup();
            assert_eq!(before, seen.len(), "{name} has duplicate opcodes");
        }
    }

    #[test]
    fn table_holds_exactly_the_151_documented_opcodes() {
        // The NMOS 6502 has 151 documented instructions. A hand-transcribed
        // table that hits that number exactly, with no duplicates, is very
        // unlikely to be wrong by omission or by accident.
        let total: usize = TABLE.iter().map(|(_, modes)| modes.len()).sum();
        assert_eq!(total, 151, "opcode count");
        assert_eq!(TABLE.len(), 56, "mnemonic count");
    }

    #[test]
    fn no_opcode_is_claimed_by_two_mnemonics() {
        let mut all: Vec<(u8, &str)> = Vec::new();
        for (name, modes) in TABLE {
            for (_, op) in *modes {
                all.push((*op, name));
            }
        }
        all.sort_unstable();
        for w in all.windows(2) {
            assert_ne!(
                w[0].0, w[1].0,
                "opcode {:#04x} claimed by both {} and {}",
                w[0].0, w[0].1, w[1].1
            );
        }
    }
}
