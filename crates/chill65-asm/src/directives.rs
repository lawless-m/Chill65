//! Directive classification and conditional-assembly conditions.
//!
//! The set implemented here is what the corpus actually uses, censused across
//! all three source trees rather than taken from a MACRO-11 manual. Condition
//! frequencies: `NE` 369, `EQ` 165, `GE` 103, `NDF` 56, `GT` 16, `NB` 12,
//! `LT` 6, `IDN` 4, `B` 2.
//!
//! `LE`, `DF` and `DIF` do not occur, but each is the one-line complement of a
//! condition that does, and MACRO-11 defines them as a set — so they are
//! implemented rather than left as a trap for the second title.

/// A directive the assembler acts on. Listing and cosmetic directives collapse
/// to [`Dir::Ignored`]: they must *parse*, but they emit nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Asect,
    Radix,
    Byte,
    Word,
    /// `.ASCII /text/` — delimiter-quoted literal text.
    Ascii,
    Asciz,
    Blkb,
    Blkw,
    Include,
    If,
    Iff,
    Ift,
    /// `.IFTF` — the rest of the conditional assembles in *both* branches.
    Iftf,
    Iif,
    Endc,
    Rept,
    /// `.IRP NAME,<a,b,c>` — repeat the body once per item, substituting NAME.
    /// One occurrence in the corpus (`HLL65F.MAC:224`) but it generates the
    /// counted shift macros `ASLS`/`LSRS`/`INXS`/... used ~100 times.
    Irp,
    /// `.IRPC NAME,<chars>` — repeat the body once per *character*, where
    /// `.IRP` repeats once per comma-separated item. Absent from Crystal
    /// Castles; Space Duel uses it fourteen times, building parallel symbol
    /// families such as `ROCK'X'1` over `<0123>`.
    Irpc,
    /// `.NCHR SYM,<text>` — define SYM as the number of characters in the
    /// bracketed argument. Absent from Crystal Castles; Space Duel uses it five
    /// times, all inside string-to-character-code macros.
    Nchr,
    Endr,
    Error,
    Globl,
    /// `.GLOBB` — like `.GLOBL`, but the symbol is byte-sized, so a reference
    /// to it from a unit that does not define it takes a zero-page operand.
    Globb,
    Enabl,
    Dsabl,
    Nocross,
    End,
    /// `.MACRO NAME params` — body captured until the matching `.ENDM`.
    Macro,
    Endm,
    /// `.DEFSTACK NAME,DEPTH` — HLL65F declares `PC` and `REGSAV` this way.
    Defstack,
    Push,
    Pop,
    /// `.GETPOINTER STACK,SYM` — SYM := the stack's remaining depth.
    GetPointer,
    /// `.VCTRS ADDR,W1,W2,...` — set the location counter and emit words.
    Vctrs,
    /// `.MEXIT` — abandon the rest of the current macro expansion.
    Mexit,
    /// Parsed and discarded: `.TITLE`, `.SBTTL`, `.PAGE`, `.LIST`, `.NLIST`,
    /// `.PRINT`, `.REM`, `.MCALL`, and the like.
    Ignored,
}

pub fn classify(name: &str) -> Option<Dir> {
    let n = name.to_ascii_uppercase();
    Some(match n.as_str() {
        ".ASECT" | ".CSECT" | ".PSECT" => Dir::Asect,
        ".RADIX" => Dir::Radix,
        ".BYTE" => Dir::Byte,
        ".ASCII" => Dir::Ascii,
        ".ASCIZ" => Dir::Asciz,
        ".WORD" => Dir::Word,
        ".BLKB" => Dir::Blkb,
        ".BLKW" => Dir::Blkw,
        ".INCLUDE" => Dir::Include,
        ".IF" => Dir::If,
        ".IFF" => Dir::Iff,
        ".IFTF" => Dir::Iftf,
        ".IFT" => Dir::Ift,
        ".IIF" => Dir::Iif,
        ".ENDC" => Dir::Endc,
        ".REPT" => Dir::Rept,
        ".IRP" => Dir::Irp,
        ".IRPC" => Dir::Irpc,
        ".NCHR" => Dir::Nchr,
        ".ENDR" => Dir::Endr,
        ".ERROR" => Dir::Error,
        ".GLOBL" => Dir::Globl,
        ".GLOBB" => Dir::Globb,
        ".ENABL" | ".ENABLE" => Dir::Enabl,
        ".DSABL" | ".DISABLE" => Dir::Dsabl,
        ".NOCROSS" | ".CROSS" => Dir::Nocross,
        ".END" => Dir::End,
        ".MACRO" => Dir::Macro,
        ".ENDM" => Dir::Endm,
        ".DEFSTACK" => Dir::Defstack,
        ".PUSH" => Dir::Push,
        ".POP" => Dir::Pop,
        ".GETPOINTER" => Dir::GetPointer,
        ".VCTRS" => Dir::Vctrs,
        ".MEXIT" => Dir::Mexit,
        ".TITLE" | ".SBTTL" | ".PAGE" | ".LIST" | ".NLIST" | ".PRINT" | ".REM" | ".MCALL"
        | ".IDENT" | ".EVEN" | ".ODD" | ".WARN" | ".NOCROSS1" => Dir::Ignored,
        _ => return None,
    })
}

/// Conditions accepted by `.IF` / `.IIF`.
///
/// They fall into three families, and the family decides how the operand is
/// read — which is why this cannot be a flat comparison:
///
/// - **numeric** (`EQ` `NE` `GT` `GE` `LT` `LE`) — evaluate an expression and
///   compare against zero, *signed*. `CG.MAC`'s guards
///   (`.IF GE,.-0F0` / `.ERROR ; RAM overlap with sounds`) depend on this: the
///   subtraction wraps when the location counter is below `0F0`, so an unsigned
///   comparison would read as "past the limit" and fire the guard backwards.
/// - **symbol** (`DF` `NDF`) — is a symbol defined?
/// - **text** (`B` `NB` `IDN` `DIF`) — inspect a macro argument's raw text,
///   not its value. `HLL65F.MAC` is built on these (`.IF NB,<COND>`,
///   `.IF IDN,<.A>,<.OR.>`), and they only become meaningful once macro
///   expansion substitutes the arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Df,
    Ndf,
    B,
    Nb,
    Idn,
    Dif,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CondKind {
    Numeric,
    Symbol,
    Text,
}

impl Cond {
    pub fn parse(s: &str) -> Option<Cond> {
        Some(match s.to_ascii_uppercase().as_str() {
            "EQ" | "Z" => Cond::Eq,
            "NE" | "NZ" => Cond::Ne,
            "GT" | "G" => Cond::Gt,
            "GE" => Cond::Ge,
            "LT" | "L" => Cond::Lt,
            "LE" => Cond::Le,
            "DF" => Cond::Df,
            "NDF" => Cond::Ndf,
            "B" => Cond::B,
            "NB" => Cond::Nb,
            "IDN" => Cond::Idn,
            "DIF" => Cond::Dif,
            _ => return None,
        })
    }

    pub fn kind(self) -> CondKind {
        match self {
            Cond::Eq | Cond::Ne | Cond::Gt | Cond::Ge | Cond::Lt | Cond::Le => CondKind::Numeric,
            Cond::Df | Cond::Ndf => CondKind::Symbol,
            Cond::B | Cond::Nb | Cond::Idn | Cond::Dif => CondKind::Text,
        }
    }

    /// Test a numeric condition. `v` is compared against zero **as signed**.
    pub fn test_numeric(self, v: u16) -> bool {
        let s = v as i16;
        match self {
            Cond::Eq => s == 0,
            Cond::Ne => s != 0,
            Cond::Gt => s > 0,
            Cond::Ge => s >= 0,
            Cond::Lt => s < 0,
            Cond::Le => s <= 0,
            _ => unreachable!("not a numeric condition"),
        }
    }

    pub fn test_symbol(self, defined: bool) -> bool {
        match self {
            Cond::Df => defined,
            Cond::Ndf => !defined,
            _ => unreachable!("not a symbol condition"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_corpus_directives() {
        assert_eq!(classify(".BYTE"), Some(Dir::Byte));
        assert_eq!(classify(".byte"), Some(Dir::Byte));
        assert_eq!(classify(".ENABLE"), Some(Dir::Enabl)); // CSTART.MAC spells it long
        assert_eq!(classify(".ENABL"), Some(Dir::Enabl)); // CRP.MAC spells it short
        assert_eq!(classify(".SBTTL"), Some(Dir::Ignored));
        assert_eq!(classify(".BOGUS"), None);
        assert_eq!(classify(".MACRO"), Some(Dir::Macro));
        assert_eq!(classify(".DEFSTACK"), Some(Dir::Defstack));
        // Macro *parameters* look like directives but must not be classified
        // as such: HLL65F names them .1. and .2.
        assert_eq!(classify(".1."), None);
        assert_eq!(classify(".2."), None);
    }

    #[test]
    fn ge_guard_needs_signed_comparison() {
        // CG.MAC:  .IF GE,.-0F0  /  .ERROR ; RAM overlap with sounds
        // Counter at 0x00E0, so .-0F0 wraps to 0xFFF0. Signed that is -16, so
        // the guard must NOT fire. Unsigned it would look like 65520 and would.
        assert!(!Cond::Ge.test_numeric(0xFFF0));
        assert!(Cond::Ge.test_numeric(0x0010));
        assert!(Cond::Lt.test_numeric(0xFFF0));
    }

    #[test]
    fn condition_families() {
        assert_eq!(Cond::parse("NE").unwrap().kind(), CondKind::Numeric);
        assert_eq!(Cond::parse("NDF").unwrap().kind(), CondKind::Symbol);
        assert_eq!(Cond::parse("NB").unwrap().kind(), CondKind::Text);
        assert_eq!(Cond::parse("IDN").unwrap().kind(), CondKind::Text);
        assert_eq!(Cond::parse("NOSUCH"), None);
    }
}
