//! The assembler's symbol table, and turning an address into a name.
//!
//! `chill65-asm --symbols` writes one line per symbol, sorted:
//!
//! ```text
//! ECC9 ROMTST [global]
//! ED17 GOTCHK [CST.MAC]
//! ```
//!
//! A four-digit hex address, the name, and the unit it belongs to in brackets —
//! `[global]` for a `NAME::` definition, otherwise the source file.
//!
//! Attribution needs the *nearest preceding* symbol: a program counter almost
//! never lands exactly on a label, it lands somewhere inside the routine that
//! label names.

use std::path::Path;

/// One symbol: address, name, and the unit that defined it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub addr: u16,
    pub name: String,
    pub unit: String,
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]", self.name, self.unit)
    }
}

/// A symbol table, sorted by address.
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    entries: Vec<Symbol>,
}

impl Symbols {
    pub fn parse(text: &str) -> Result<Symbols, String> {
        let mut entries = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut fields = line.splitn(3, ' ');
            let (Some(addr), Some(name), Some(unit)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(format!("line {}: cannot parse {line:?}", i + 1));
            };
            let addr = u16::from_str_radix(addr, 16)
                .map_err(|_| format!("line {}: {addr:?} is not a hex address", i + 1))?;
            let unit = unit
                .trim()
                .strip_prefix('[')
                .and_then(|u| u.strip_suffix(']'))
                .ok_or_else(|| format!("line {}: expected [unit], found {unit:?}", i + 1))?;
            entries.push(Symbol {
                addr,
                name: name.to_string(),
                unit: unit.to_string(),
            });
        }
        entries.sort_by_key(|s| s.addr);
        Ok(Symbols { entries })
    }

    pub fn load(path: &Path) -> Result<Symbols, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Symbols::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The symbol at or before `addr` — the routine an address falls inside.
    ///
    /// `None` only when `addr` precedes every symbol in the table.
    pub fn nearest(&self, addr: u16) -> Option<&Symbol> {
        let mut i = match self.entries.binary_search_by_key(&addr, |s| s.addr) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        // Several symbols can share an address. Rewind to the first of them, so
        // that every address in a routine's range reports the same name and the
        // answer does not depend on which index the search happened to land on.
        let found = self.entries[i].addr;
        while i > 0 && self.entries[i - 1].addr == found {
            i -= 1;
        }
        Some(&self.entries[i])
    }

    /// The routine `addr` falls inside, without an offset.
    ///
    /// This is the unit attribution aggregates by. Counting per instruction
    /// would scatter one routine's blame across every store it makes — the
    /// attract screen's border alone is drawn by four consecutive addresses —
    /// and bury the answer the report exists to give.
    pub fn routine_for(&self, addr: u16) -> String {
        match self.nearest(addr) {
            Some(s) => s.to_string(),
            None => format!("${addr:04X}"),
        }
    }

    /// How `addr` should be named when the exact instruction matters.
    pub fn name_for(&self, addr: u16) -> String {
        match self.nearest(addr) {
            Some(s) if s.addr == addr => s.to_string(),
            Some(s) => format!("{s}+{}", addr - s.addr),
            None => format!("${addr:04X}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
A000 START [global]
A010 DRAW [CDB.MAC]
A010 DRAWALIAS [CDB.MAC]
ECC9 ROMTST [CST.MAC]
";

    fn syms() -> Symbols {
        Symbols::parse(SAMPLE).expect("parse")
    }

    #[test]
    fn parses_addresses_names_and_units() {
        let s = syms();
        assert_eq!(s.len(), 4);
        let first = s.nearest(0xA000).unwrap();
        assert_eq!(first.name, "START");
        assert_eq!(first.unit, "global");
        assert_eq!(first.to_string(), "START [global]");
    }

    #[test]
    fn nearest_finds_the_containing_routine() {
        let s = syms();
        assert_eq!(s.nearest(0xA000).unwrap().name, "START");
        assert_eq!(s.nearest(0xA00F).unwrap().name, "START", "still inside START");
        assert_eq!(s.nearest(0xA011).unwrap().name, "DRAW");
        assert_eq!(s.nearest(0xFFFF).unwrap().name, "ROMTST");
    }

    #[test]
    fn addresses_below_every_symbol_have_no_owner() {
        assert!(syms().nearest(0x0000).is_none());
    }

    #[test]
    fn duplicate_addresses_resolve_to_the_first() {
        // Two names share A010; the answer must not depend on sort stability.
        assert_eq!(syms().nearest(0xA010).unwrap().name, "DRAW");
    }

    #[test]
    fn names_carry_an_offset_when_inside_a_routine() {
        let s = syms();
        // routine_for drops the offset; name_for keeps it.
        assert_eq!(s.routine_for(0xA014), "DRAW [CDB.MAC]");
        assert_eq!(s.name_for(0xA010), "DRAW [CDB.MAC]");
        assert_eq!(s.name_for(0xA014), "DRAW [CDB.MAC]+4");
        assert_eq!(s.name_for(0x0001), "$0001");
    }

    #[test]
    fn malformed_lines_are_rejected() {
        assert!(Symbols::parse("ZZZZ NAME [unit]\n").is_err());
        assert!(Symbols::parse("A000 NAME unit\n").is_err());
        assert!(Symbols::parse("A000\n").is_err());
    }
}
