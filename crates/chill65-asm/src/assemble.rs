//! Two-pass assembly driver and output.
//!
//! Pass one builds the symbol table and sizes everything; pass two emits bytes
//! into an address-keyed image, so that overlapping writes from `.=` motion
//! behave as the original assembler did — **last write wins**. That resolution
//! is not a guess: it reproduces Atari's own documented ROM checksums for all
//! thirteen ROMs across three revisions (`integrity.md` §2).
//!
//! The location counter is tracked *across* `.INCLUDE` boundaries — region
//! context crosses files, which is what `tools/writescan.py` had to learn
//! before `EECOIN` could be classified correctly.
//!
//! Instruction encoding (task 11) and macro expansion (task 10) are not here
//! yet; line-leading symbols that are neither directive nor label are counted
//! as unhandled and skipped, so this stage can be exercised on its own.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::directives::{classify, Cond, CondKind, Dir};
use crate::encode::{self, is_instruction};
use crate::macros::{parse_params, split_args, MacroDef};
use crate::expr::{self, Context, Eval};
use crate::lexer::{Lexer, Mode, Tok, Token};

/// Supplies source for `.INCLUDE`. A directory in real use, a map in tests.
pub trait SourceProvider {
    fn load(&self, name: &str) -> Option<Vec<u8>>;
}

/// Loads includes from a directory, trying the name as written and then with
/// `.MAC` appended — `CSTART.MAC` writes `.INCLUDE HLL65F`, others write
/// `.INCLUDE CMAC.MAC`.
pub struct DirProvider {
    pub root: std::path::PathBuf,
}

impl SourceProvider for DirProvider {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        for cand in [name.to_string(), format!("{name}.MAC")] {
            let p = self.root.join(&cand);
            if let Ok(b) = std::fs::read(&p) {
                return Some(b);
            }
        }
        None
    }
}

impl SourceProvider for HashMap<String, String> {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        self.get(name)
            .or_else(|| self.get(&format!("{name}.MAC")))
            .map(|s| s.as_bytes().to_vec())
    }
}

#[derive(Clone, Debug)]
struct CondFrame {
    /// Is this branch currently emitting?
    active: bool,
    /// The result of this frame's `.IF` test.
    ///
    /// `.IFT` / `.IFF` / `.IFTF` are *subconditionals* on that one result —
    /// "if true", "if false", "if true or false" — not an if/elseif chain, and
    /// they may appear repeatedly within a single frame. `CCN.MAC` does exactly
    /// that: `.IF EQ,MECHS-1` at line 315 is followed by `.IFF` (323),
    /// `.IFTF` (344), `.IFT` (370), `.IFF` (372) and `.IFTF` (374), all
    /// referring back to the same test.
    ///
    /// Modelling this with a "has any branch been taken" flag makes the second
    /// `.IFT` activate on the strength of the earlier `.IFF` having fired,
    /// which assembles both arms of a two-way choice.
    cond_true: bool,
    /// Was the enclosing context emitting? A nested `.IF` inside a dead branch
    /// stays dead whatever its own condition says.
    outer_active: bool,
}

pub struct Assembler<'a> {
    provider: &'a dyn SourceProvider,
    /// Symbols private to the current assembly unit — defined with a single
    /// colon or a single `=`. Cleared between units.
    pub locals: HashMap<String, u16>,
    /// Symbols shared across units — defined with `::` or `==`. Persist for the
    /// whole build, which is what lets `CRF` reference `RS.KEY::` from `CRP`.
    pub globals: HashMap<String, u16>,
    pub global_decls: HashSet<String>,
    /// The subset of `global_decls` written `.GLOBB` rather than `.GLOBL`.
    ///
    /// `.GLOBB` says the symbol is byte-sized — it lives in the zero page — so
    /// a unit that references it without defining it can size the operand short
    /// even though it cannot see the value. That is the whole point of the
    /// directive, and it is why it is not a synonym for `.GLOBL`.
    pub byte_globals: HashSet<String>,
    /// The subset of `byte_globals` the **current unit** declares itself.
    ///
    /// `.ENABL AMA` confines the byte hint to this set: see `byte_decls`'s use
    /// in the operand sizing below. Restored from `unit_byte_decls` at unit
    /// start, so a declaration late in a unit still governs a use early in it.
    pub byte_decls: HashSet<String>,
    pub image: BTreeMap<u16, u8>,
    loc: u16,
    radix: u32,
    /// `.ENABL AMA` — auto zero-page management. Recorded here; the encoder
    /// (task 11) is what acts on it.
    pub ama: bool,
    /// `.ENABL M68` — 6800 byte order, high byte first, for `.WORD`.
    m68: bool,
    cond: Vec<CondFrame>,
    pass: u8,
    ended: bool,
    errors: Vec<String>,
    /// Non-fatal diagnostics: `.ERROR` and unrecognised directives.
    pub warnings: Vec<String>,
    /// Every store instruction's resolved target, recorded post-expansion.
    /// This is the authoritative self-modifying-code check that `smc-gate.md`
    /// promised: the Phase 0 scan could only read source text.
    pub store_targets: Vec<(String, u16, String)>,

    /// The recorded IR, filled during pass two. See `crate::ir`.
    pub ir: crate::ir::Ir,
    /// Unit currently being assembled, for IR provenance.
    ir_unit: String,
    /// A run of non-instruction bytes being accumulated: `(addr, len)`.
    ir_data: Option<(u16, u16)>,
    /// True while an instruction's own bytes are being emitted, so `emit` does
    /// not mistake them for data.
    ir_in_instruction: bool,
    /// How often each macro was invoked, post-expansion.
    pub macro_uses: HashMap<String, usize>,
    /// Branch instructions emitted from inside an HLL65F expansion.
    pub branches_structured: usize,
    /// Branch instructions written by hand, outside any macro.
    pub branches_direct: usize,
    /// Branch instructions from some other macro (M6502's BINT, INC16, ...).
    pub branches_other_macro: usize,
    /// Line-leading symbols that are neither directive, label, macro nor
    /// instruction. Should be zero on a clean build.
    pub unhandled: usize,
    depth: usize,

    /// Macros defined so far. Populated by `.MACRO`, and by macros that
    /// themselves contain `.MACRO` — which is how HLL65F's `DEFIF`/`DEFEND`
    /// generate the `IFEQ`/`PLEND` families.
    pub macros: HashMap<String, MacroDef>,
    /// `.DEFSTACK` stacks, keyed by name. HLL65F uses `PC` and `REGSAV`.
    stacks: HashMap<String, Vec<u16>>,
    /// Declared depth per `.DEFSTACK`, for `.GETPOINTER`.
    stack_sizes: HashMap<String, u16>,
    /// Counter for `?` generated labels.
    gensym: u32,
    /// The macro expansion chain, for the recursion guard's error message.
    expanding: Vec<String>,
    /// Set by `.MEXIT`; abandons the rest of the current expansion.
    mexit: bool,
    /// Addressing mode chosen at each instruction site during pass one, replayed
    /// in pass two.
    ///
    /// Sizing must not differ between passes. With AMA a forward-referenced
    /// operand is unresolved in pass one, so the mode is chosen as absolute
    /// then; by pass two the symbol is known and a fresh decision could pick
    /// zero page instead, shortening the instruction and shifting every address
    /// after it. Recording pass one's choice makes the two passes agree by
    /// construction.
    modes: Vec<Mode>,
    mode_idx: usize,
    /// Each unit's private symbols, kept between passes. Pass one populates a
    /// unit's table; pass two must start from it, or every forward reference
    /// within the unit resolves in pass one and then fails in pass two.
    pub unit_locals: HashMap<String, HashMap<String, u16>>,
    /// Symbols the *current* unit defines, private or global.
    ///
    /// Only these may be sized as zero page. A reference to anything else is an
    /// external, and the original assembler — running one unit at a time with
    /// no knowledge of the others — had no choice but to size it absolute.
    /// Assembling everything together resolves such symbols early, and AMA
    /// would then shrink the instruction from three bytes to two and shift
    /// every address after it.
    defined_here: HashSet<String>,
    /// Every name each unit defines anywhere in it, carried from the probe.
    ///
    /// Whether a symbol is an import is a property of the **whole unit**, not
    /// of how far pass one has read. `COIN65.MAC` uses `$CNCT` some forty lines
    /// before the base-page block that defines it; consulting `defined_here` at
    /// the point of use called it an import and sized it absolute, where the
    /// original assembled `65 26` — zero page.
    pub unit_defined: HashMap<String, HashSet<String>>,
    /// Every name each unit declares `.GLOBL`/`.GLOBB` anywhere in it, carried
    /// from the probe.
    ///
    /// Like `unit_defined`, and for the same reason: `define` asks whether the
    /// name has been declared global, and that is a fact about the unit rather
    /// than about how far pass one has read. `AS2DEC.MAC`'s HEAD macro writes
    /// the assignment first and the declaration two lines later —
    /// `NAME'8 = ...` then `.GLOBL NAME'8` — so every symbol it defines was
    /// invisible to the other modules.
    pub unit_global_decls: HashMap<String, HashSet<String>>,
    /// Every name each unit declares `.GLOBB` anywhere in it, carried from the
    /// probe — the `.GLOBB`-only counterpart of `unit_global_decls`.
    pub unit_byte_decls: HashMap<String, HashSet<String>>,
    /// Current local-label region. `N$` labels are scoped between ordinary
    /// labels, so the same `10$` recurs all through the corpus.
    local_scope: u32,

    // ---- relocatable sections ----
    //
    // Crystal Castles has no `.CSECT` at all, so none of this is reachable on
    // that corpus and its output cannot move. Space Duel links fifteen modules,
    // most of them relocatable, and `SDGEN1.COM`'s order is the link order.
    /// The section being assembled into, or `None` for the absolute section.
    section: Option<String>,
    /// The absolute location counter, parked while a `.CSECT` is open.
    abs_loc: u16,
    /// Section names in first-encounter order — the order they are laid out in.
    pub sec_order: Vec<String>,
    /// Each section's running offset, carried **across units** so that two
    /// modules contributing to one section concatenate rather than overlap.
    sec_off: HashMap<String, u16>,
    /// The high-water offset each section reached: its size.
    pub sec_size: HashMap<String, u16>,
    /// Where each section was placed. Empty during the sizing probe.
    pub sec_base: HashMap<String, u16>,
    /// True while measuring sizes, before any base is known.
    probing: bool,
}

/// The name given to `.CSECT` with no operand.
///
/// `AS2TST.MAC` opens a bare `.CSECT` *and* a named one, so "no operand" cannot
/// be read as "absolute" — it is a section in its own right, distinct from
/// every named one.
const UNNAMED_SECTION: &str = "~blank";

/// The blank section belongs to one unit, so each gets its own.
///
/// A named `.CSECT` is shared — two modules opening `AS2MSG` are appending to
/// one region. The blank one is not: `AS2COI`'s blank content sits at `741A`
/// and `A2GOOF`'s at `8F43`, two and a half kilobytes apart, each immediately
/// after the module that precedes it in link order.
fn blank_section(unit: &str) -> String {
    format!("{UNNAMED_SECTION}@{unit}")
}

/// Where the relocatable sections begin.
///
/// Derived, not chosen. Assembling each module alone and searching `ASTRD2.LDA`
/// for its longest byte runs locates it: `AST2RT` at `6EE5`, `AS2SAC` `703C`,
/// `AS2POK` `70C0`, `AS2MSG` `7730`, `AS2TST` `7FAB`, `A2IRQ` `8639` on four
/// independent agreeing runs, `VGUTR2` `8E42`. Those rise monotonically in
/// `SDGEN1.COM`'s link order, which is concatenation; running the measured
/// sizes back from the first anchored module gives this origin. `inventory.md`
/// §7 item 3l records the measurement.
///
/// Running the measured sizes back from each anchor gives the origin it
/// implies, and the first three agree exactly: `AST2RT` `6EE5 - 0189`,
/// `AS2SAC` `703C - 02E0`, `AS2POK` `70C0 - 0364`, all `6D5C`. It then predicts
/// `A2NAME` at `741A`, which is an address the oracle confirms independently.
/// Anchors further down the chain imply other origins, but that is accumulated
/// error in the sizes before them, not disagreement about where the region
/// starts — an origin is only as good as every size preceding it.
const SECTION_ORIGIN: u16 = 0x6D5C;

/// The base every section is given during the sizing probe.
///
/// Any value at or above `0x100` does: sizes come from offsets, not from where
/// the probe happened to put things, and the only thing the base can influence
/// is whether an operand looks zero-page. Since the real bases are all well
/// above `0x100` too, the probe and the real run agree on every operand width,
/// which is the property that makes a two-stage layout sound.
const PROBE_BASE: u16 = 0x8000;

type Line<'t> = &'t [Token];

impl<'a> Assembler<'a> {
    pub fn new(provider: &'a dyn SourceProvider) -> Self {
        Assembler {
            provider,
            locals: HashMap::new(),
            globals: HashMap::new(),
            global_decls: HashSet::new(),
            byte_globals: HashSet::new(),
            byte_decls: HashSet::new(),
            image: BTreeMap::new(),
            loc: 0,
            radix: 16,
            ama: false,
            m68: false,
            cond: Vec::new(),
            pass: 1,
            ended: false,
            errors: Vec::new(),
            warnings: Vec::new(),
            store_targets: Vec::new(),
            ir: crate::ir::Ir::default(),
            ir_unit: String::new(),
            ir_data: None,
            ir_in_instruction: false,
            macro_uses: HashMap::new(),
            branches_structured: 0,
            branches_direct: 0,
            branches_other_macro: 0,
            unhandled: 0,
            depth: 0,
            macros: HashMap::new(),
            stacks: HashMap::new(),
            stack_sizes: HashMap::new(),
            gensym: 0,
            expanding: Vec::new(),
            mexit: false,
            modes: Vec::new(),
            mode_idx: 0,
            local_scope: 0,
            unit_locals: HashMap::new(),
            defined_here: HashSet::new(),
            unit_defined: HashMap::new(),
            unit_global_decls: HashMap::new(),
            unit_byte_decls: HashMap::new(),
            section: None,
            abs_loc: 0,
            sec_order: Vec::new(),
            sec_off: HashMap::new(),
            sec_size: HashMap::new(),
            sec_base: HashMap::new(),
            probing: false,
        }
    }

    /// Assemble one root in two passes.
    pub fn assemble(&mut self, root_name: &str) -> Result<BTreeMap<u16, u8>, Vec<String>> {
        self.assemble_units(&[root_name])
    }

    /// Assemble several units into one image.
    ///
    /// The original build assembled `CRF`, `C99`, `CRP` and `CLS` separately and
    /// linked the results. That separation was a tooling constraint, not a
    /// semantic one — what it bought was a private namespace per unit. We give
    /// each unit its own private symbol table, macro table and assembler state,
    /// share only the `::`/`==` globals and the image, and so need no link step.
    ///
    /// Both passes run over every unit, so a unit may reference a global that a
    /// later unit defines — which is exactly what `CRF` does with `CRP`'s
    /// `RS.KEY::`. Pass one sizes such a reference as absolute (it is
    /// unresolved at the time), and pass two replays that decision, matching
    /// what a separate assembly plus linker would have produced.
    ///
    /// # Relocatable sections
    ///
    /// A `.CSECT` has no address of its own; the linker decides where it goes,
    /// and it cannot decide until it knows how big every section is. So when
    /// any section is present the work is done twice: a throwaway probe on a
    /// fresh assembler measures the sections, [`Self::place_sections`] lays them
    /// out, and the real run assembles against those bases. Crystal Castles has
    /// no `.CSECT` anywhere, so the probe never runs there and its output cannot
    /// move.
    pub fn assemble_units(&mut self, roots: &[&str]) -> Result<BTreeMap<u16, u8>, Vec<String>> {
        // The probe runs for every build, not only those with sections: it also
        // establishes which names each unit defines *anywhere*, which pass one
        // cannot know as it goes.
        if !self.probing {
            let mut probe = Assembler::new(self.provider);
            probe.probing = true;
            let _ = probe.assemble_units(roots);
            if !probe.sec_order.is_empty() {
                self.place_sections(&probe.sec_order, &probe.sec_size);
            }
            self.unit_defined = std::mem::take(&mut probe.unit_defined);
            self.unit_global_decls = std::mem::take(&mut probe.unit_global_decls);
            self.unit_byte_decls = std::mem::take(&mut probe.unit_byte_decls);
        }
        for pass in 1..=2 {
            // The layout is fixed; the counters that produce it are not, and
            // both passes must walk them identically.
            self.section = None;
            self.abs_loc = 0;
            self.sec_off.clear();
            self.sec_size.clear();
            self.pass = pass;
            self.image.clear();
            self.errors.clear();
            self.warnings.clear();
            self.store_targets.clear();
            self.ir = crate::ir::Ir::default();
            self.ir_data = None;
            self.ir_in_instruction = false;
            self.macro_uses.clear();
            self.branches_structured = 0;
            self.branches_direct = 0;
            self.branches_other_macro = 0;
            self.unhandled = 0;
            self.mode_idx = 0;
            // Reset per PASS, not per unit: both passes must mint the same
            // generated-label names in the same order, or pass two references
            // labels that pass one never defined.
            self.gensym = 0;
            if pass == 1 {
                self.modes.clear();
            }

            for root_name in roots {
                // Per-unit state. Globals and the image deliberately persist,
                // and so do the section offsets — that is what makes two
                // modules contributing to one section concatenate rather than
                // land on top of each other. Each unit starts absolute.
                //
                // A unit begins in the **blank section**, not the absolute
                // one: `.ASECT` is an explicit switch into absolute, and
                // MACRO-11's default is relocatable. Most modules say `.ASECT`
                // in their first few lines and so are absolute in practice, but
                // `AS2COI.MAC` and `A2GOOF.MAC` never declare a section at all
                // — treating them as absolute piled them at `0000`, on top of
                // each other and over nothing the oracle has.
                self.abs_loc = 0;
                //
                // The blank section is **per unit**, unlike a named one. Two
                // modules that both fall into it are not contributing to a
                // shared region: `AS2COI` is the sixth module and the oracle
                // places its blank content at `741A`, right after `AS2POK`,
                // while `A2GOOF` is the fifteenth and sits at `8F43`, right
                // after `VGUTR2` — the far end of the image. Merging them put
                // `A2GOOF`'s sixteen bytes at `741A` too and pushed everything
                // after it sixteen bytes late.
                let blank = blank_section(root_name);
                self.section = Some(blank.clone());
                let base = self.sec_base.get(&blank).copied().unwrap_or(PROBE_BASE);
                self.loc = base.wrapping_add(self.sec_off.get(&blank).copied().unwrap_or(0));
                self.radix = 16;
                self.ama = false;
                self.m68 = false;
                self.cond.clear();
                self.ended = false;
                self.macros.clear();
                self.stacks.clear();
                self.stack_sizes.clear();
                self.expanding.clear();
                self.mexit = false;
                self.local_scope = 0;
                // Start from what the probe found this unit defines anywhere,
                // then keep adding as we go — the probe has nothing to say on
                // its own first pass, and this way the set only ever grows.
                self.defined_here = self
                    .unit_defined
                    .get(*root_name)
                    .cloned()
                    .unwrap_or_default();
                self.locals = self
                    .unit_locals
                    .get(*root_name)
                    .cloned()
                    .unwrap_or_default();
                self.global_decls = self
                    .unit_global_decls
                    .get(*root_name)
                    .cloned()
                    .unwrap_or_default();
                self.byte_decls = self
                    .unit_byte_decls
                    .get(*root_name)
                    .cloned()
                    .unwrap_or_default();
                // `byte_globals` is deliberately NOT cleared. A `.GLOBB`
                // declares that the symbol *is a byte*, which is a fact about
                // the symbol rather than about the unit that mentions it, and
                // the linker carries it across the whole program. `$CNCT` is
                // declared `.GLOBB` in `ASTRD2.MAC` and used unprefixed by
                // `COIN65.MAC` forty lines from any declaration of its own; the
                // original assembles that use as `65 26`, zero page.
                //
                // Crystal Castles is the control and stays exact: its one
                // `.GLOBB` names `$INTCT` and `ATRACT`, and `SN.NUM` — declared
                // only `.GLOBL` — must stay absolute.

                self.ir_unit = root_name.to_string();

                let Some(raw) = self.provider.load(root_name) else {
                    self.errors
                        .push(format!("cannot open root source {root_name}"));
                    continue;
                };
                self.run_source(root_name, &raw);
                // Whatever section the unit ended in, record where it got to.
                self.park_section();
                self.unit_defined
                    .insert(root_name.to_string(), self.defined_here.clone());
                self.unit_global_decls
                    .insert(root_name.to_string(), self.global_decls.clone());
                self.unit_byte_decls
                    .insert(root_name.to_string(), self.byte_decls.clone());
                self.unit_locals
                    .insert(root_name.to_string(), self.locals.clone());
            }
        }
        if self.errors.is_empty() {
            // Operands are read back from the finished image: HLL65F branches
            // are emitted with a self-pointing placeholder and patched later.
            // See `Ir::resolve_operands_from`.
            self.flush_ir_data();
            let image = std::mem::take(&mut self.image);
            self.ir.resolve_operands_from(&image);
            self.ir.segment_routines();
            Ok(image)
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    fn run_source(&mut self, file: &str, raw: &[u8]) {
        if self.depth > 32 {
            self.errors.push(format!("include nesting too deep at {file}"));
            return;
        }
        let toks = Lexer::new(file.to_string()).tokenise_bytes(raw);
        let lines: Vec<&[Token]> = split_lines(&toks);
        self.run_lines(&lines);
    }

    fn run_lines(&mut self, lines: &[Line]) {
        let mut i = 0usize;
        while i < lines.len() {
            if self.ended || self.mexit {
                return;
            }
            let consumed = self.run_line(lines, i);
            i += consumed.max(1);
        }
    }

    fn active(&self) -> bool {
        self.cond.last().map(|c| c.active).unwrap_or(true)
    }

    /// Process one line. Returns how many lines were consumed (more than one
    /// only for `.REPT`, which swallows its body).
    fn run_line(&mut self, lines: &[Line], idx: usize) -> usize {
        let line = lines[idx];
        let mut t = 0usize;

        // Skip a leading label, but only define it when emitting.
        while let Some(tok) = line.get(t) {
            match &tok.tok {
                Tok::LabelDef { name, global } => {
                    if self.active() {
                        // A non-local label closes the previous local-label
                        // region: MACRO-11 scopes `N$` between normal labels,
                        // so `10$` may be reused freely down the file.
                        //
                        // Synthetic labels are exempt. A macro that mints its
                        // own label (`INC16 ADDR,?B` ends its body with `B:`)
                        // must not split the *caller's* local-label region —
                        // doing so orphaned every `N$` reference that spanned
                        // such an expansion.
                        if !name.contains('~') {
                            self.local_scope += 1;
                        }
                        if self.pass == 1 {
                            self.define(name, self.loc, *global);
                        } else {
                            // Pass two, where `loc` is final: record the label
                            // for the IR. Only address labels are recorded —
                            // `=` assignments are constants, not places, and
                            // routine segmentation wants places.
                            self.flush_ir_data();
                            let unit = self.ir_unit.clone();
                            let at = self.loc;
                            self.ir.events.push(crate::ir::Event::Label(crate::ir::Label {
                                addr: at,
                                name: name.to_string(),
                                scope: if *global {
                                    crate::ir::Scope::Global
                                } else {
                                    crate::ir::Scope::Unit
                                },
                                unit,
                            }));
                        }
                        if *global && self.pass == 1 {
                            self.global_decls.insert(intern(name));
                        }
                    }
                    t += 1;
                }
                Tok::LocalLabelDef(n) => {
                    if self.active() && self.pass == 1 {
                        let key = self.local_key(*n);
                        self.defined_here.insert(key.clone());
                        self.locals.insert(key, self.loc);
                    }
                    t += 1;
                }
                _ => break,
            }
        }

        let rest = &line[t..];
        if rest.is_empty() || matches!(rest[0].tok, Tok::Eol | Tok::Comment(_)) {
            return 1;
        }

        // `.= expr` — location counter assignment.
        if matches!(rest[0].tok, Tok::Dot) && matches!(rest.get(1).map(|x| &x.tok), Some(Tok::Assign { .. }))
        {
            if self.active() {
                if let Some(v) = self.eval_here(&rest[2..]) {
                    self.loc = v;
                }
            }
            return 1;
        }

        // `NAME = expr` / `NAME == expr`
        if let (Tok::Symbol(name), Some(Tok::Assign { global })) =
            (&rest[0].tok, rest.get(1).map(|x| &x.tok))
        {
            if self.active() {
                if let Some(v) = self.eval_here(&rest[2..]) {
                    self.define(name, v, *global);
                }
                if *global && self.pass == 1 {
                    self.global_decls.insert(intern(name));
                }
            }
            return 1;
        }

        let Tok::Symbol(head) = &rest[0].tok else {
            // A statement whose operator field holds an expression rather than
            // an opcode or directive is an implicit `.WORD` list. `CEN.MAC:1803`
            // writes the enemy-state jump table that way:
            //
            //     1$: 10$-1,11$-1,12$-1,13$-1,14$-1,15$-1
            //         16$-1,17$-1,18$-1,19$-1
            //         20$-1,21$-1,22$-1,23$-1,24$-1,25$-1,26$-1
            //
            // Seventeen entries, 34 bytes — and silently emitting nothing for
            // them shifted every later address by exactly that much.
            if self.active() {
                for field in split_commas(rest) {
                    let v = self.eval_here(field).unwrap_or(0);
                    self.emit(v as u8);
                    self.emit((v >> 8) as u8);
                }
            }
            return 1;
        };

        // Directives win over macros, so a macro cannot shadow `.BYTE`.
        if let Some(d) = classify(head) {
            return self.directive(d, rest, lines, idx);
        }
        if !self.active() {
            return 1;
        }
        if self.macros.contains_key(&head.to_ascii_uppercase()) {
            self.invoke_macro(&head.clone(), &rest[1..]);
            return 1;
        }
        if is_instruction(head) {
            self.instruction(&head.clone(), &rest[1..]);
            return 1;
        }
        self.unhandled += 1;
        if self.pass == 2 {
            // A dot-prefixed name is a directive we do not implement. The
            // original tolerated them — `CRP.MAC:56` carries `.GLOBB`, a typo
            // for `.GLOBL` that shipped, and which is load-bearing precisely
            // *because* it failed: had it worked, CRP's private `$INTCT` and
            // `ATRACT` would have gone global and collided with `CG.MAC`'s.
            // Report, do not abort.
            let msg = format!("{:?}: unknown mnemonic {head}", rest[0].span);
            if head.starts_with('.') {
                self.warnings.push(msg);
            } else {
                self.errors.push(msg);
            }
        }
        1
    }

    /// Expand a macro and run its body through the normal line processor, so
    /// directives inside macro bodies need no special handling.
    fn invoke_macro(&mut self, name: &str, args: &[Token]) {
        let key = name.to_ascii_uppercase();
        if self.expanding.len() >= 64 {
            let chain = self.expanding.join(" -> ");
            self.errors
                .push(format!("macro expansion too deep: {chain} -> {name}"));
            self.mexit = true;
            return;
        }
        let Some(def) = self.macros.get(&key).cloned() else {
            return;
        };
        let argv = split_args(args);
        let body = match def.expand(&argv, &mut self.gensym) {
            Ok(b) => b,
            Err(e) => {
                self.errors.push(format!("expanding {name}: {}", e.message));
                return;
            }
        };
        *self.macro_uses.entry(key.clone()).or_insert(0) += 1;
        if self.pass == 2 {
            // Structure markers are recorded where the construct is invoked,
            // which is the only place the structure is visible: after expansion
            // an IFEQ is an ordinary BNE.
            // Only the outermost construct is recorded. `ENDIF` expands to
            // `THEN` (codegen-readiness.md notes they are the same construct),
            // and `IFEQ` to `IFXX`, so a source-level construct can invoke
            // another. Recording both would report two closes for one open and
            // leave the emitter with unbalanced structure.
            let nested = self
                .expanding
                .iter()
                .any(|m| crate::ir::Marker::classify(m).is_some());
            if let Some(marker) = crate::ir::Marker::classify(&key).filter(|_| !nested) {
                self.flush_ir_data();
                let unit = self.ir_unit.clone();
                let addr = self.loc;
                self.ir
                    .events
                    .push(crate::ir::Event::Marker { marker, addr, unit });
            }
        }
        self.expanding.push(name.to_string());
        let refs: Vec<Line> = body.iter().map(|l| l.as_slice()).collect();
        self.run_lines(&refs);
        self.expanding.pop();
        // .MEXIT ends this expansion only, not the enclosing one.
        self.mexit = false;
    }

    /// Encode one 6502 instruction and emit it.
    ///
    /// Two operand syntaxes coexist. The explicit prefix form (`LDA I,FROM`,
    /// `STA NY,PKPTR`) is what `OPC65.MAC` defines, and the abbreviations
    /// `#`, `@` and `(X)`/`(Y)` are what `M6502.MAC`'s header tells its users
    /// to write instead:
    ///
    /// > Use the abbreviations #,@,(X), i.e.
    /// >   a: TRAM ARG1(X) @ARG2(Y)     works, but
    /// >   b: TRAM <X,ARG1> <NY,ARG2>   doesn't.
    fn instruction(&mut self, mnemonic: &str, operand: &[Token]) {
        let site = operand
            .first()
            .map(|t| format!("{:?}", t.span))
            .unwrap_or_else(|| "<no operand>".into());
        let mut rest: &[Token] = operand;
        let mut written: Option<Mode> = None;

        // Leading prefix, `#` (immediate) or `@` (indirect).
        match rest.first().map(|t| &t.tok) {
            Some(Tok::Prefix(m)) => {
                written = Some(*m);
                rest = &rest[1..];
            }
            Some(Tok::Punct('#')) => {
                written = Some(Mode::I);
                rest = &rest[1..];
            }
            Some(Tok::Punct('@')) => {
                written = Some(Mode::N);
                rest = &rest[1..];
            }
            _ => {}
        }

        // Trailing `(X)` / `(Y)` index. Combined with a leading `@` this is the
        // indirect-indexed form: `@CT.ROM(Y)` is `(CT.ROM),Y`.
        let body_end = rest
            .iter()
            .position(|t| matches!(t.tok, Tok::Eol | Tok::Comment(_)))
            .unwrap_or(rest.len());
        let body = &rest[..body_end];
        let mut index: Option<Mode> = None;
        let mut index_len = 0usize;
        let n = body.len();
        let as_index = |t: &Tok| match t {
            Tok::Symbol(s) if s.eq_ignore_ascii_case("X") => Some(Mode::X),
            Tok::Symbol(s) if s.eq_ignore_ascii_case("Y") => Some(Mode::Y),
            _ => None,
        };
        if n >= 3
            && matches!(body[n - 3].tok, Tok::Punct('('))
            && matches!(body[n - 1].tok, Tok::Punct(')'))
        {
            index = as_index(&body[n - 2].tok);
            index_len = 3;
        } else if n >= 2 && matches!(body[n - 2].tok, Tok::Punct('(')) {
            // Unclosed index. `CEN.MAC` writes `STA EN.IY(X` and `LDA EN.HP(X`
            // with no closing paren — a typo present in all three revisions
            // that the original assembler tolerated, since the shipped ROMs
            // work. Its operand scan simply ends at the line.
            index = as_index(&body[n - 1].tok);
            index_len = 2;
        }
        if index.is_none() {
            index_len = 0;
        }
        let body = &body[..n - index_len];
        if let Some(ix) = index {
            written = Some(match (written, ix) {
                (Some(Mode::N), Mode::Y) => Mode::Ny,
                (Some(Mode::N), Mode::X) => Mode::Nx,
                _ => ix,
            });
        }
        let rest = body;

        let has_operand = !rest.is_empty();
        let value = if has_operand { self.eval_here(rest) } else { None };

        // A symbol this unit declares `.GLOBL` is an EXTERNAL: its value is
        // supplied by another module, so the assembler cannot know it fits in
        // the zero page and must size the operand absolute.
        //
        // The discriminator is the *declaration*, not whether the symbol
        // happens to be resolvable. `CRP.MAC:52` declares
        // `.GLOBL TUNTAB,DOTPL,EN.HEI,SN.NUM`, and the oracle assembles
        // `STA SN.NUM` (00BA) absolute — `8d ba 00`. Two lines earlier,
        // `CRP.MAC:56` declares `.GLOBB $INTCT,ATRACT` — a typo that never took
        // effect — and the oracle assembles `LDA ATRACT` (00B9) and
        // `LDA $INTCT` (00A3) as *zero page*, `a5 b9` and `a5 a3`.
        //
        // So the misspelling changed instruction sizes, and reproducing the
        // original byte-for-byte means reproducing its consequences.
        // Both conditions are needed: a unit that declares `.GLOBL X` *and*
        // defines `X` itself is exporting it, not importing it, and sizes it
        // normally. `CG.MAC` does exactly that for `$INTCT` — declares it in
        // `CCN.MAC` and defines it at `CG.MAC:351` — so within CRF it is a
        // local. Only a declaration with no local definition is an import.
        let external = rest.iter().any(|t| match &t.tok {
            Tok::Symbol(n) => {
                let k = intern(n);
                self.global_decls.contains(&k) && !self.defined_here.contains(&k)
            }
            _ => false,
        });
        let size_value = if external { None } else { value };

        // ...unless the declaration was `.GLOBB`, which says the import is
        // byte-sized. Then the unit *can* size it short without seeing the
        // value, and that is the only reason to write `.GLOBB` instead of
        // `.GLOBL`.
        //
        // How far that hint reaches depends on `.ENABL AMA`. A unit's own
        // `.GLOBB` always applies. A `.GLOBB` made in some *other* unit reaches
        // this one only while AMA is off — under AMA an import the unit has not
        // itself declared byte-sized is assembled absolute.
        //
        // Measured, not assumed. `A2EARO.MAC` enables AMA at line 4, declares
        // `.GLOBL ... GAME` at line 28, and never declares `GAME` byte-sized;
        // the sole `.GLOBB GAME` in the build is `A2NAME.MAC:21`. The original
        // assembles all three of `A2EARO`'s plain uses (`:386`, `:405`, `:408`)
        // absolute. `AS2COI.MAC` enables AMA nowhere, declares only
        // `.GLOBL ... $CNCT` (via `COIN65.MAC:125`), and the original assembles
        // its plain `$CNCT` uses zero page — so the cross-unit hint does carry,
        // and it is AMA that withdraws it.
        //
        // The hint is not withdrawn wholesale under AMA: `A2EARO` declares
        // `.GLOBB TEMP7,ATRACT,LANG,FRAME,...` itself and the original sizes
        // every one of those zero page, which is why the test is per-unit
        // rather than simply `!self.ama`.
        //
        // Crystal Castles is the control and stays exact: its one `.GLOBB`
        // names `$INTCT` and `ATRACT` and sits in the same unit that uses them,
        // so the own-declaration arm covers it either way.
        let byte_external = rest.iter().any(|t| match &t.tok {
            Tok::Symbol(n) => {
                let k = intern(n);
                if self.defined_here.contains(&k) {
                    return false;
                }
                self.byte_decls.contains(&k) || (!self.ama && self.byte_globals.contains(&k))
            }
            _ => false,
        });

        // Choose the mode once, in pass one, and replay it in pass two.
        let mode = if self.pass == 1 {
            match encode::resolve_mode(
                mnemonic,
                written,
                has_operand,
                size_value,
                self.ama,
                byte_external,
            ) {
                Ok(m) => m,
                Err(e) => {
                    self.errors.push(format!("{site}: {mnemonic}: {e}"));
                    Mode::A
                }
            }
        } else {
            match self.modes.get(self.mode_idx) {
                Some(m) => *m,
                None => {
                    self.errors
                        .push(format!("{mnemonic}: pass mismatch at instruction {}", self.mode_idx));
                    Mode::A
                }
            }
        };
        if self.pass == 1 {
            self.modes.push(mode);
        }
        self.mode_idx += 1;

        if self.pass == 1 {
            // Size only; the operand may still be unresolved.
            self.loc = self.loc.wrapping_add(encode::size_of(mode));
            return;
        }

        let up = mnemonic.to_ascii_uppercase();
        if matches!(up.as_str(), "BCC" | "BCS" | "BEQ" | "BMI" | "BNE" | "BPL" | "BVC" | "BVS") {
            // Attribute the branch to its origin. HLL65F emits its branches
            // from inside IFXX / ..END / ELSE, so a branch whose expansion
            // chain touches one of those is structured control flow; anything
            // at top level is a hand-written branch and is what a relooper
            // would have to recover (plan section 4).
            const HLL: &[&str] = &[
                "IFXX", "FND", "ELSE", "THEN", "ENDIF", "ENDC", "BEGIN", "LOC", "..END",
                "CONTINUE", "IF",
            ];
            if self.expanding.is_empty() {
                self.branches_direct += 1;
            } else if self
                .expanding
                .iter()
                .any(|m| HLL.contains(&m.to_ascii_uppercase().as_str()))
            {
                self.branches_structured += 1;
            } else {
                self.branches_other_macro += 1;
            }
        }
        if matches!(
            up.as_str(),
            "STA" | "STX" | "STY" | "INC" | "DEC" | "ASL" | "LSR" | "ROL" | "ROR"
        ) {
            if let Some(v) = value {
                if !matches!(mode, Mode::Ac | Mode::I) {
                    let chain = if self.expanding.is_empty() {
                        "<direct>".to_string()
                    } else {
                        self.expanding.join(">")
                    };
                    self.store_targets.push((up.clone(), v, chain));
                }
            }
        }

        // Close any run of data bytes before the instruction's own bytes, so
        // the IR reads in emission order.
        self.flush_ir_data();
        let ir_addr = self.loc;
        self.ir_in_instruction = true;

        match encode::encode(mnemonic, mode, value, self.loc) {
            Ok(bytes) => {
                let size = bytes.len() as u16;
                for b in bytes {
                    self.emit(b);
                }
                if self.pass == 2 {
                    let unit = self.ir_unit.clone();
                    self.ir
                        .events
                        .push(crate::ir::Event::Instruction(crate::ir::Instruction {
                            addr: ir_addr,
                            mnemonic: up.clone(),
                            mode,
                            value,
                            operand_text: operand_text(operand),
                            size,
                            chain: self.expanding.clone(),
                            unit,
                            routine: None,
                        }));
                }
            }
            Err(e) => {
                self.errors.push(format!("{site}: {mnemonic}: {e}"));
                self.loc = self.loc.wrapping_add(encode::size_of(mode));
            }
        }
        self.ir_in_instruction = false;
    }

    /// Capture a `.MACRO` body up to its matching `.ENDM`, nesting-aware.
    ///
    /// Nesting matters: `DEFIF` and `DEFEND` contain an inner `.MACRO`, which is
    /// how the `IFEQ`/`PLEND` families come into being when they are expanded.
    fn define_macro(&mut self, args: Line, lines: &[Line], idx: usize) -> usize {
        let mut depth = 1usize;
        let mut j = idx + 1;
        while j < lines.len() {
            let d = leading_directive(lines[j]);
            if opens_block(d) {
                depth += 1;
            } else if closes_block(d) {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            j += 1;
        }
        if depth != 0 {
            self.errors.push(".MACRO without matching .ENDM".into());
            return lines.len() - idx;
        }

        if self.active() {
            if let Some(Tok::Symbol(name)) = args.first().map(|t| &t.tok) {
                let def = MacroDef {
                    name: name.clone(),
                    params: parse_params(&args[1..]),
                    body: lines[idx + 1..j].iter().map(|l| l.to_vec()).collect(),
                };
                self.macros.insert(name.to_ascii_uppercase(), def);
            }
        }
        j - idx + 1
    }

    fn directive(&mut self, d: Dir, rest: Line, lines: &[Line], idx: usize) -> usize {
        let args = &rest[1..];

        // Conditional structure is processed even inside a dead branch, so that
        // nesting stays balanced.
        match d {
            Dir::If => {
                let outer = self.active();
                let take = outer && self.test_condition(args).unwrap_or(false);
                if self.pass == 2 {
                    if let Ok(want) = std::env::var("CHILL65_TRACE_IF") {
                        let at = rest.first().map(|t| format!("{:?}", t.span)).unwrap_or_default();
                        if at.contains(&want) {
                            let txt: String = args
                                .iter()
                                .map(|t| crate::macros::token_text(t))
                                .collect::<Vec<_>>()
                                .join(" ");
                            eprintln!(
                                "  IF  {at}  depth={} outer={outer} take={take}   {txt}",
                                self.cond.len()
                            );
                        }
                    }
                }
                let cond_true = take;
                self.cond.push(CondFrame {
                    active: cond_true,
                    cond_true,
                    outer_active: outer,
                });
                return 1;
            }
            Dir::Iff => {
                if self.pass == 2 {
                    if let Ok(want) = std::env::var("CHILL65_TRACE_IF") {
                        let at = rest.first().map(|t| format!("{:?}", t.span)).unwrap_or_default();
                        if at.contains(&want) {
                            eprintln!("  IFF {at}  depth={}", self.cond.len());
                        }
                    }
                }
                if let Some(f) = self.cond.last_mut() {
                    f.active = f.outer_active && !f.cond_true;
                } else {
                    self.errors.push(".IFF without .IF".into());
                }
                return 1;
            }
            Dir::Iftf => {
                if self.pass == 2 {
                    if let Ok(want) = std::env::var("CHILL65_TRACE_IF") {
                        let at = rest.first().map(|t| format!("{:?}", t.span)).unwrap_or_default();
                        if at.contains(&want) {
                            eprintln!("  IFTF {at}  depth={}", self.cond.len());
                        }
                    }
                }
                // Assembles in both branches: re-enable regardless of which way
                // the condition went, without disturbing `taken`.
                if let Some(f) = self.cond.last_mut() {
                    f.active = f.outer_active;
                } else {
                    self.errors.push(".IFTF without .IF".into());
                }
                return 1;
            }
            Dir::Ift => {
                if self.pass == 2 {
                    if let Ok(want) = std::env::var("CHILL65_TRACE_IF") {
                        let at = rest.first().map(|t| format!("{:?}", t.span)).unwrap_or_default();
                        if at.contains(&want) {
                            eprintln!("  IFT {at}  depth={}", self.cond.len());
                        }
                    }
                }
                if let Some(f) = self.cond.last_mut() {
                    f.active = f.outer_active && f.cond_true;
                } else {
                    self.errors.push(".IFT without .IF".into());
                }
                return 1;
            }
            Dir::Endc => {
                if self.pass == 2 {
                    if let Ok(want) = std::env::var("CHILL65_TRACE_IF") {
                        let at = rest.first().map(|t| format!("{:?}", t.span)).unwrap_or_default();
                        if at.contains(&want) {
                            eprintln!("  ENDC {at}  depth={}", self.cond.len());
                        }
                    }
                }
                if self.cond.pop().is_none() {
                    self.errors.push(".ENDC without .IF".into());
                }
                return 1;
            }
            Dir::Rept => return self.repeat_block(args, lines, idx),
            Dir::Irp => return self.irp_block(args, lines, idx, false),
            Dir::Irpc => return self.irp_block(args, lines, idx, true),
            Dir::Macro => return self.define_macro(args, lines, idx),
            Dir::Endm => return 1, // consumed by define_macro
            Dir::Endr => {
                // A stray .ENDR; the matching .REPT consumes its own.
                return 1;
            }
            _ => {}
        }

        if !self.active() {
            return 1;
        }

        match d {
            Dir::Nocross | Dir::Ignored => {}

            Dir::Asect => self.enter_section(None),
            Dir::Csect => {
                let name = match args.iter().find_map(|t| match &t.tok {
                    Tok::Symbol(s) => Some(s.to_ascii_uppercase()),
                    _ => None,
                }) {
                    Some(n) => n,
                    // A bare `.CSECT` *is* the blank section, and the blank
                    // section belongs to this unit.
                    None => blank_section(&self.ir_unit),
                };
                self.enter_section(Some(name));
            }

            Dir::Nchr => self.nchr(args),

            Dir::Radix => {
                // The operand of .RADIX is always decimal, whatever the current
                // radix — otherwise `.RADIX 10` under hex would mean sixteen.
                if let Some(Tok::Number { text, .. }) = args.first().map(|t| &t.tok) {
                    if let Ok(r) = text.parse::<u32>() {
                        if (2..=16).contains(&r) {
                            self.radix = r;
                        } else {
                            self.errors.push(format!("unsupported radix {r}"));
                        }
                    }
                }
            }

            Dir::Ascii => {
                for t in args {
                    if let Tok::Str(text) = &t.tok {
                        for ch in text.bytes() {
                            self.emit(ch);
                        }
                    }
                }
            }

            Dir::Byte => {
                for field in split_commas(args) {
                    let v = self.eval_here(field).unwrap_or(0);
                    self.emit(v as u8);
                }
            }

            Dir::Word => {
                for field in split_commas(args) {
                    let v = self.eval_here(field).unwrap_or(0);
                    if self.m68 {
                        // 6800 order: high byte first. CRP.MAC's LDAH depends
                        // on this to pull the high byte of a symbol address.
                        self.emit((v >> 8) as u8);
                        self.emit(v as u8);
                    } else {
                        self.emit(v as u8);
                        self.emit((v >> 8) as u8);
                    }
                }
            }

            Dir::Blkb | Dir::Blkw => {
                let n = self.eval_here(args).unwrap_or(0);
                let step = if d == Dir::Blkw { 2 } else { 1 };
                self.loc = self.loc.wrapping_add(n.wrapping_mul(step));
            }

            Dir::Include => {
                if let Some(Tok::Symbol(name)) = args.first().map(|t| &t.tok) {
                    let name = name.clone();
                    match self.provider.load(&name) {
                        Some(raw) => {
                            self.depth += 1;
                            self.run_source(&name, &raw);
                            self.depth -= 1;
                        }
                        None => self.errors.push(format!("cannot find include {name}")),
                    }
                }
            }

            Dir::Iif => {
                // .IIF cond,arg,statement — a one-line .IF/.ENDC.
                let fields = split_commas(args);
                if fields.len() >= 2 {
                    // Rebuild `cond,arg` *with* its comma: test_condition splits
                    // on commas itself, so concatenating the fields bare would
                    // present them as a single field and silently test nothing.
                    let mut head: Vec<Token> = fields[0].to_vec();
                    head.push(comma_token());
                    head.extend_from_slice(fields[1]);

                    // Take the statement as the raw tail after the second
                    // top-level comma rather than rejoining split fields —
                    // splitting drops trailing comments, and `.ERROR`'s message
                    // lives in one:
                    //     .IIF GT,...S0-127.,.ERROR ...S0 ; BRANCH OUT OF RANGE
                    let mut depth = 0i32;
                    let mut commas = 0;
                    let mut tail = None;
                    for (i, t) in args.iter().enumerate() {
                        match t.tok {
                            Tok::Punct('<') => depth += 1,
                            Tok::Punct('>') => depth -= 1,
                            Tok::Punct(',') if depth == 0 => {
                                commas += 1;
                                if commas == 2 {
                                    tail = Some(i + 1);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(start) = tail {
                        if self.test_condition(&head).unwrap_or(false) {
                            let stmt: Vec<Token> = args[start..].to_vec();
                            let stmt_lines: Vec<Line> = vec![&stmt];
                            self.run_line(&stmt_lines, 0);
                        }
                    }
                }
            }

            Dir::Error => {
                // MACRO-11 form is `.ERROR expr ; message`. HLL65F relies on
                // both halves: `.ERROR ...S0 ; BRANCH OUT OF RANGE` reports the
                // offending offset *and* says what is wrong.
                let expr_toks: Vec<Token> = args
                    .iter()
                    .take_while(|t| !matches!(t.tok, Tok::Comment(_)))
                    .cloned()
                    .collect();
                let message: String = args
                    .iter()
                    .find_map(|t| match &t.tok {
                        Tok::Comment(c) => Some(c.trim().to_string()),
                        _ => None,
                    })
                    .unwrap_or_default();
                if self.pass == 2 {
                    // `.ERROR` reports; it does not abort. `CRF.MAC:72` carries
                    // an unconditional `.error ; dummy` in shipped source, so a
                    // fatal reading would make the real program unbuildable.
                    let value = if expr_toks.is_empty() {
                        String::new()
                    } else {
                        match self.eval_here(&expr_toks) {
                            Some(v) => format!(" ({} / {v:#06x})", v as i16),
                            None => String::new(),
                        }
                    };
                    self.warnings.push(format!(".ERROR: {message}{value}"));
                }
            }

            Dir::Globl | Dir::Globb => {
                // Both passes. `define` consults this set, and a `NAME = expr`
                // definition runs in both passes — collecting declarations in
                // pass one alone would file such a symbol as shared on the way
                // through and private on the way back.
                for f in split_commas(args) {
                    if let Some(Tok::Symbol(s)) = f.first().map(|t| &t.tok) {
                        self.global_decls.insert(intern(s));
                        if d == Dir::Globb {
                            self.byte_globals.insert(intern(s));
                            self.byte_decls.insert(intern(s));
                        }
                    }
                }
            }

            Dir::Enabl | Dir::Dsabl => {
                let on = d == Dir::Enabl;
                for f in split_commas(args) {
                    if let Some(Tok::Symbol(s)) = f.first().map(|t| &t.tok) {
                        match s.to_ascii_uppercase().as_str() {
                            "AMA" => self.ama = on,
                            "M68" => self.m68 = on,
                            _ => {} // LC and friends do not affect codegen
                        }
                    }
                }
            }

            Dir::End => self.ended = true,
            Dir::Mexit => self.mexit = true,

            // `.DEFSTACK NAME,DEPTH` — the depth is advisory; the stack grows.
            Dir::Defstack => {
                let fields = split_commas(args);
                if let Some(Tok::Symbol(n)) = fields.first().and_then(|f| f.first()).map(|t| &t.tok)
                {
                    let name = n.to_ascii_uppercase();
                    self.stacks.entry(name.clone()).or_default();
                    let size = fields.get(1).and_then(|f| self.eval_here(f)).unwrap_or(0);
                    self.stack_sizes.insert(name, size);
                }
            }

            // `.GETPOINTER STACK,SYM` — SYM := remaining depth. `HLL65F`'s
            // `HLL65` macro uses it to assert the PC stack came back balanced:
            //     ...1=0 / .GETPOINTER PC,...1
            //     .IIF NE,..STK$-...1,.ERROR ..STK$-...1 ; STACK INBALANCE
            // so a balanced build reports the declared depth and stays quiet.
            Dir::GetPointer => {
                let fields = split_commas(args);
                let stack = fields
                    .first()
                    .and_then(|f| f.first())
                    .and_then(|t| match &t.tok {
                        Tok::Symbol(s) => Some(s.to_ascii_uppercase()),
                        _ => None,
                    });
                let sym = fields.get(1).and_then(|f| f.first()).and_then(|t| match &t.tok {
                    Tok::Symbol(s) => Some(s.clone()),
                    _ => None,
                });
                if let (Some(stack), Some(sym)) = (stack, sym) {
                    let used = self.stacks.get(&stack).map(|s| s.len()).unwrap_or(0) as u16;
                    let size = self.stack_sizes.get(&stack).copied().unwrap_or(0);
                    self.define(&sym, size.wrapping_sub(used), false);
                }
            }

            // `.VCTRS ADDR,W1,W2,...` — position at ADDR and lay down words.
            // `CRF.MAC:77` writes `.VCTRS 0FFF8,INTER,INTER,START,INTER`, and
            // the shipped ROM has FFFC->E000 (START) and FFFE->E009 (INTER).
            Dir::Vctrs => {
                let fields = split_commas(args);
                if let Some(addr) = fields.first().and_then(|f| self.eval_here(f)) {
                    // The address is absolute, so the counter it sets must be
                    // the absolute one. Writing it into a section's counter
                    // left the section apparently stretching from its base up
                    // to the vectors: `AS2TST.MAC` ends with
                    // `.VCTRS 8FFA,...` while still inside `.CSECT AS2TST`,
                    // and the section measured `0x9000 - base` instead of its
                    // real content.
                    //
                    // Crystal Castles is unaffected — `CRF.MAC:77` does the
                    // same thing with no section open anywhere in the corpus,
                    // where parking the absolute counter and restoring it is a
                    // no-op.
                    self.enter_section(None);
                    self.loc = addr;
                    for f in &fields[1..] {
                        let v = self.eval_here(f).unwrap_or(0);
                        self.emit(v as u8);
                        self.emit((v >> 8) as u8);
                    }
                }
            }

            // `.PUSH NAME,sym,sym,...` pushes those symbols' current values in
            // the order written; `.POP NAME,sym,...` assigns them back, so a
            // pop list written in reverse restores the originals.
            Dir::Push | Dir::Pop => {
                let fields = split_commas(args);
                let Some(stack_name) = fields.first().and_then(|f| match f.first().map(|t| &t.tok) {
                    Some(Tok::Symbol(s)) => Some(s.to_ascii_uppercase()),
                    _ => None,
                }) else {
                    return 1;
                };
                let names: Vec<String> = fields[1..]
                    .iter()
                    .filter_map(|f| match f.first().map(|t| &t.tok) {
                        Some(Tok::Symbol(s)) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                for n in names {
                    if d == Dir::Push {
                        let v = self.lookup(&n).unwrap_or(0);
                        self.stacks.entry(stack_name.clone()).or_default().push(v);
                    } else {
                        let v = self
                            .stacks
                            .get_mut(&stack_name)
                            .and_then(|st| st.pop())
                            .unwrap_or_else(|| {
                                if self.pass == 2 {
                                    // A pop from an empty stack means the
                                    // structure macros are unbalanced.
                                    0
                                } else {
                                    0
                                }
                            });
                        self.locals.insert(intern(&n), v);
                    }
                }
            }

            Dir::Asciz => {
                for t in args {
                    if let Tok::Str(text) = &t.tok {
                        for ch in text.bytes() {
                            self.emit(ch);
                        }
                    }
                }
                self.emit(0);
            }

            Dir::If | Dir::Iff | Dir::Ift | Dir::Endc | Dir::Rept | Dir::Irp | Dir::Irpc
            | Dir::Endr | Dir::Macro | Dir::Endm | Dir::Iftf => unreachable!(),
        }
        1
    }

    /// `.REPT count` … `.ENDR`, nesting-aware.
    ///
    /// The body is located by looking **only** at each line's leading token.
    /// `CCN.MAC` wraps pages of documentation prose in `.REPT 0`, and that prose
    /// is not valid syntax — it must never be parsed, only skipped.
    fn repeat_block(&mut self, args: Line, lines: &[Line], idx: usize) -> usize {
        let count = if self.active() {
            self.eval_here(args).unwrap_or(0)
        } else {
            0
        };

        let mut depth = 1usize;
        let mut j = idx + 1;
        while j < lines.len() {
            let d = leading_directive(lines[j]);
            if opens_block(d) {
                depth += 1;
            } else if closes_block(d) {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            j += 1;
        }
        if depth != 0 {
            let at = args.first().map(|t| format!("{:?}", t.span)).unwrap_or_default();
            let chain = if self.expanding.is_empty() {
                "<top level>".to_string()
            } else {
                self.expanding.join(" -> ")
            };
            self.errors.push(format!(
                "{at}: .REPT without matching .ENDR (in {chain}, {} body lines)",
                lines.len()
            ));
            return lines.len() - idx;
        }

        let body: Vec<Line> = lines[idx + 1..j].to_vec();
        for _ in 0..count {
            if self.ended {
                break;
            }
            self.run_lines(&body);
        }
        j - idx + 1
    }

    /// `.IRP NAME,<a,b,c>` … `.ENDR` — run the body once per item with NAME
    /// bound to it. Shares `.ENDR` with `.REPT`, so the scanner counts both.
    /// `.NCHR SYM,<text>` — define SYM as the character count of the bracketed
    /// argument.
    ///
    /// The count is over the argument's **raw text**, as `.IRPC`'s is: `<0123>`
    /// lexes as one number, so counting an evaluated value would give 1 where
    /// the answer is 4. `token_text` recovers each token's source spelling, and
    /// `Tok::Number` keeps its digits verbatim.
    ///
    /// The name and the argument arrive the same way `.IRP`'s do, including the
    /// case where the name is spelled like an addressing mode and the lexer has
    /// already swallowed the separating comma — see `irp_block`.
    fn nchr(&mut self, args: Line) {
        if !self.active() {
            return;
        }
        let fields = split_commas(args);
        let (name, list): (Option<String>, Vec<Token>) =
            match fields.first().and_then(|f| f.first()).map(|t| &t.tok) {
                Some(Tok::Symbol(s)) => (
                    Some(s.clone()),
                    fields.get(1).map(|f| f.to_vec()).unwrap_or_default(),
                ),
                Some(Tok::Prefix(m)) => (Some(m.name().to_string()), fields[0][1..].to_vec()),
                _ => (None, Vec::new()),
            };
        let Some(name) = name else {
            self.errors.push(".NCHR needs a symbol to define".into());
            return;
        };
        let text: String = list
            .iter()
            .filter(|t| !matches!(t.tok, Tok::Punct('<') | Tok::Punct('>')))
            .map(crate::macros::token_text)
            .collect();
        self.define(&name, text.chars().count() as u16, false);
    }

    /// `.IRP` and `.IRPC`. They differ only in how the item list is cut:
    /// `.IRP` takes comma-separated items, `.IRPC` one character each.
    fn irp_block(&mut self, args: Line, lines: &[Line], idx: usize, per_char: bool) -> usize {
        let mut depth = 1usize;
        let mut j = idx + 1;
        while j < lines.len() {
            let d = leading_directive(lines[j]);
            if opens_block(d) {
                depth += 1;
            } else if closes_block(d) {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            j += 1;
        }
        if depth != 0 {
            let which = if per_char { ".IRPC" } else { ".IRP" };
            self.errors.push(format!("{which} without matching .ENDR"));
            return lines.len() - idx;
        }

        if self.active() {
            let fields = split_commas(args);
            // `.IRP X,<...>` names its parameter after an addressing mode, so
            // the lexer classified `X,` as a mode prefix and swallowed the
            // separating comma — the same collision `macros.rs` already
            // recovers from in `.MACRO CMPIN A,B`. When that happens the name
            // and the item list are left in one field instead of two.
            //
            // Every `.IRP` in Crystal Castles names its parameter `OPC`, which
            // is why this went unnoticed: the whole directive silently expanded
            // to nothing only for single-letter mode names, and Space Duel's
            // `.IRPC X,<0123>` is the first use that hits it.
            let (name, list): (Option<String>, Vec<Token>) =
                match fields.first().and_then(|f| f.first()).map(|t| &t.tok) {
                    Some(Tok::Symbol(s)) => (
                        Some(s.clone()),
                        fields.get(1).map(|f| f.to_vec()).unwrap_or_default(),
                    ),
                    Some(Tok::Prefix(m)) => {
                        (Some(m.name().to_string()), fields[0][1..].to_vec())
                    }
                    _ => (None, Vec::new()),
                };
            if let Some(name) = name {
                // The item list arrives as one bracketed field; split it.
                let items: Vec<Vec<Token>> = {
                    let inner: Vec<Token> = list
                        .iter()
                        .filter(|t| !matches!(t.tok, Tok::Punct('<') | Tok::Punct('>')))
                        .cloned()
                        .collect();
                    if per_char {
                        // One iteration per character of the argument's raw
                        // text. `Tok::Number` keeps its digits verbatim, so
                        // `<0123>` yields "0", "1", "2", "3" rather than the
                        // single value those digits would evaluate to.
                        //
                        // A blank between two tokens is a character of that
                        // text like any other, and joining the tokens without
                        // it loses an iteration. `AS2ROM.MAC:1097` writes
                        // `ALPHA <MCMLXXX ATARI IN>`, whose two spaces map
                        // through `VGMC.MAC`'s `JSRL CHAR.'...X` onto the blank
                        // glyph `CHAR.:` at `VGAN.MAC:129`. Dropping them emits
                        // sixteen words where the original emits eighteen.
                        //
                        // `space_before` is a flag, not a count, so a run of
                        // several blanks would still yield one. No such run
                        // exists in either corpus; widening it would be guessing
                        // at a case with no evidence behind it.
                        match inner.first() {
                            Some(first) => {
                                let text: String = inner
                                    .iter()
                                    .enumerate()
                                    .map(|(i, t)| {
                                        let sep =
                                            if i > 0 && t.space_before { " " } else { "" };
                                        format!("{sep}{}", crate::macros::token_text(t))
                                    })
                                    .collect();
                                text.chars()
                                    .map(|c| {
                                        vec![Token {
                                            tok: Tok::Symbol(c.to_string()),
                                            span: first.span.clone(),
                                            space_before: false,
                                        }]
                                    })
                                    .collect()
                            }
                            None => Vec::new(),
                        }
                    } else {
                        split_commas(&inner).into_iter().map(|x| x.to_vec()).collect()
                    }
                };
                let template = MacroDef {
                    name: format!("<.IRP{} {name}>", if per_char { "C" } else { "" }),
                    params: vec![crate::macros::Param {
                        name: name.clone(),
                        generated: false,
                    }],
                    body: lines[idx + 1..j].iter().map(|l| l.to_vec()).collect(),
                };
                for item in items {
                    if self.ended {
                        break;
                    }
                    if let Ok(body) = template.expand(&[item], &mut self.gensym) {
                        let refs: Vec<Line> = body.iter().map(|l| l.as_slice()).collect();
                        self.run_lines(&refs);
                    }
                }
            }
        }
        j - idx + 1
    }

    fn test_condition(&mut self, args: Line) -> Option<bool> {
        let fields = split_commas(args);
        let name = match fields.first().and_then(|f| f.first()) {
            Some(t) => match &t.tok {
                Tok::Symbol(s) => s.clone(),
                _ => return None,
            },
            None => return None,
        };
        let Some(cond) = Cond::parse(&name) else {
            self.errors.push(format!("unknown .IF condition {name}"));
            return None;
        };

        match cond.kind() {
            CondKind::Numeric => {
                let operand = fields.get(1)?;
                let v = self.eval_here(operand)?;
                Some(cond.test_numeric(v))
            }
            CondKind::Symbol => {
                let operand = fields.get(1)?;
                let sym = operand.iter().find_map(|t| match &t.tok {
                    Tok::Symbol(s) => Some(s.clone()),
                    _ => None,
                })?;
                Some(cond.test_symbol(self.lookup(&sym).is_some()))
            }
            CondKind::Text => {
                let a = fields.get(1).map(|f| strip_angles(f)).unwrap_or_default();
                let b = fields.get(2).map(|f| strip_angles(f)).unwrap_or_default();
                Some(match cond {
                    Cond::B => a.is_empty(),
                    Cond::Nb => !a.is_empty(),
                    Cond::Idn => a == b,
                    Cond::Dif => a != b,
                    _ => unreachable!(),
                })
            }
        }
    }

    /// Lay the measured sections out end to end from [`SECTION_ORIGIN`], in
    /// first-encounter order.
    ///
    /// First-encounter order is link order: the roots are assembled in
    /// `SDGEN1.COM`'s sequence, so a section is met when the first module
    /// contributing to it is read. The measured bases rise monotonically in
    /// that same order (`inventory.md` §7 item 3l), which is what concatenation
    /// looks like from the outside.
    pub fn place_sections(&mut self, order: &[String], sizes: &HashMap<String, u16>) {
        let mut addr = SECTION_ORIGIN;
        for name in order {
            self.sec_base.insert(name.clone(), addr);
            addr = addr.wrapping_add(sizes.get(name).copied().unwrap_or(0));
        }
    }

    /// Park the current section's offset and its high-water mark.
    ///
    /// Both matter. The offset is where the *next* contribution to this section
    /// resumes, which is what makes two modules concatenate; the high-water
    /// mark is the section's size, which is what decides where the next section
    /// starts.
    fn park_section(&mut self) {
        let Some(cur) = self.section.clone() else {
            self.abs_loc = self.loc;
            return;
        };
        let base = self.sec_base.get(&cur).copied().unwrap_or(PROBE_BASE);
        let off = self.loc.wrapping_sub(base);
        self.sec_off.insert(cur.clone(), off);
        // A section joins the layout when something is actually *in* it, not
        // when a directive naming it goes by. Every unit begins in the blank
        // section (see the per-unit reset), so registering on entry would put
        // the blank one first in every build that has any section at all;
        // registering on content puts it where its first contribution lands.
        // `AS2COI.MAC` is the case that settles it — it declares no section
        // anywhere, and the oracle has it at `741A`, which is exactly where
        // `AS2POK` ends.
        if off > 0 && !self.sec_order.contains(&cur) {
            self.sec_order.push(cur.clone());
        }
        let high = self.sec_size.entry(cur).or_insert(0);
        if off > *high {
            *high = off;
        }
    }

    /// Switch location counters. `None` is the absolute section.
    fn enter_section(&mut self, name: Option<String>) {
        self.park_section();
        match name {
            Some(n) => {
                let base = self.sec_base.get(&n).copied().unwrap_or(PROBE_BASE);
                let off = self.sec_off.get(&n).copied().unwrap_or(0);
                self.loc = base.wrapping_add(off);
                self.section = Some(n);
            }
            None => {
                self.loc = self.abs_loc;
                self.section = None;
            }
        }
    }

    /// Define a symbol in the private or shared table.
    ///
    /// `NAME:` and `NAME=` are private to the unit; `NAME::` and `NAME==` are
    /// shared. The distinction is already carried on the token — it is the
    /// source's own scoping rule, not one we invent.
    ///
    /// A declaration shares a symbol too. In MACRO-11 it is the *pair* —
    /// `.GLOBL X` somewhere in the unit and a definition of `X` in the same
    /// unit — that exports it; `X::` is shorthand for writing both at once.
    /// Crystal Castles only ever uses the shorthand, so honouring it alone was
    /// enough for four years. Space Duel separates them everywhere:
    /// `ASTRD2.MAC` declares its scratch page with `.GLOBB` at the top of the
    /// file and defines it a couple of hundred lines later as plain
    /// `TEMP2: .BLKB 2`, so under the shorthand-only rule almost nothing it
    /// publishes reached the other fourteen modules.
    fn define(&mut self, name: &str, value: u16, global: bool) {
        let key = intern(name);
        self.defined_here.insert(key.clone());
        if global || self.global_decls.contains(&key) {
            self.globals.insert(key, value);
        } else {
            self.locals.insert(key, value);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<u16> {
        let key = intern(name);
        self.locals
            .get(&key)
            .or_else(|| self.globals.get(&key))
            .copied()
    }

    fn local_key(&self, n: u32) -> String {
        format!("~L{}${n}", self.local_scope)
    }

    /// Rewrite local-label references into their scoped symbol names before
    /// evaluation, so `BNE 10$` resolves to this region's `10$`.
    fn resolve_locals(&self, toks: Line) -> Vec<Token> {
        toks.iter()
            .map(|t| match &t.tok {
                Tok::LocalLabel(n) => Token {
                    tok: Tok::Symbol(self.local_key(*n)),
                    span: t.span.clone(),
                    space_before: t.space_before,
                },
                _ => t.clone(),
            })
            .collect()
    }

    fn eval_here(&mut self, toks: Line) -> Option<u16> {
        let owned;
        let toks = if toks.iter().any(|t| matches!(t.tok, Tok::LocalLabel(_))) {
            owned = self.resolve_locals(toks);
            &owned[..]
        } else {
            toks
        };
        let scope = Scope {
            locals: &self.locals,
            globals: &self.globals,
        };
        let ctx = Context::new(self.radix, Some(self.loc), &scope);
        match expr::eval(toks, &ctx) {
            Ok(Eval::Value(v)) => Some(v),
            Ok(Eval::Unresolved(name)) => {
                if self.pass == 2 {
                    self.errors.push(format!("undefined symbol {name}"));
                }
                None
            }
            Err(e) => {
                if self.pass == 2 {
                    self.errors.push(e.to_string());
                }
                None
            }
        }
    }

    fn emit(&mut self, b: u8) {
        if self.pass == 2 {
            self.image.insert(self.loc, b);
            if !self.ir_in_instruction {
                // Anything not emitted by `instruction` is data: a directive,
                // a table, storage. Runs are coalesced so a 4K table is one
                // event rather than four thousand.
                match self.ir_data {
                    Some((addr, len)) if addr.wrapping_add(len) == self.loc => {
                        self.ir_data = Some((addr, len + 1));
                    }
                    _ => {
                        self.flush_ir_data();
                        self.ir_data = Some((self.loc, 1));
                    }
                }
            }
        }
        self.loc = self.loc.wrapping_add(1);
    }

    /// Close any open run of data bytes and record it.
    fn flush_ir_data(&mut self) {
        if let Some((addr, len)) = self.ir_data.take() {
            let unit = self.ir_unit.clone();
            self.ir.events.push(crate::ir::Event::Data(crate::ir::Data {
                addr,
                len,
                chain: self.expanding.clone(),
                unit,
            }));
        }
    }
}

/// The operand as it was written, flattened back to a string.
///
/// For reading the IR, and for the emitter to quote in a comment beside the
/// code it generates — an address alone is a poor thing to debug against.
fn operand_text(operand: &[Token]) -> String {
    let mut out = String::new();
    for t in operand {
        match &t.tok {
            Tok::Symbol(n) => out.push_str(n),
            Tok::Number { text, .. } => out.push_str(text),
            Tok::Punct(c) => out.push(*c),
            Tok::Prefix(m) => out.push_str(match m {
                Mode::I => "#",
                _ => "@",
            }),
            other => out.push_str(&format!("{other:?}")),
        }
    }
    out
}

fn comma_token() -> Token {
    Token {
        tok: Tok::Punct(','),
        span: crate::lexer::Span {
            file: "<synthetic>".into(),
            line: 0,
            col: 0,
        },
        space_before: false,
    }
}

fn split_lines(toks: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, t) in toks.iter().enumerate() {
        if matches!(t.tok, Tok::Eol) {
            out.push(&toks[start..i]);
            start = i + 1;
        }
    }
    if start < toks.len() {
        out.push(&toks[start..]);
    }
    out
}

/// The directive a line begins with, skipping any label. Used for `.REPT`
/// body scanning, where the contents must not be parsed.
fn leading_directive(line: Line) -> Option<Dir> {
    for t in line {
        match &t.tok {
            Tok::LabelDef { .. } | Tok::LocalLabelDef(_) => continue,
            Tok::Symbol(s) => return classify(s),
            _ => return None,
        }
    }
    None
}



/// Intern a source symbol name.
///
/// MACRO-11 stores symbols RAD50-encoded — three characters per 16-bit word,
/// **six characters maximum** — and silently truncates anything longer.
/// `PST65.MAC` encodes every opcode name with `.RAD50 /NAME/`, and the whole
/// corpus obeys the limit: of 914 defined symbols, 658 are exactly six
/// characters and none is longer. The truncation is load-bearing, not
/// cosmetic — `CCUBE.MAC:6` defines `CLENGT` and `CCUBE.MAC:200` references
/// `CLENGTH`, which resolves only because the seventh character is discarded.
///
/// Names containing `~` are synthetic (generated `?` labels and scoped local
/// labels). `~` cannot occur in a source symbol, so they are exempt — they are
/// already unique and truncating them would make them collide.
/// Resolves a name against a unit's private table first, then the shared one.
///
/// The order is what keeps `$INTCT` straight: `CG.MAC:351` defines `$INTCT::`
/// (shared) while `CRP.MAC:158` defines `$INTCT:` (private to the sound
/// package). Inside CRP the private one must win; everywhere else the shared
/// one is the only candidate. They are genuinely different variables in
/// different zero pages.
struct Scope<'a> {
    locals: &'a HashMap<String, u16>,
    globals: &'a HashMap<String, u16>,
}

impl expr::Symbols for Scope<'_> {
    fn lookup(&self, name: &str) -> Option<u16> {
        let key = intern(name);
        self.locals
            .get(&key)
            .or_else(|| self.globals.get(&key))
            .copied()
    }
}

fn intern(name: &str) -> String {
    let up = name.to_ascii_uppercase();
    if up.contains('~') {
        return up;
    }
    up.chars().take(6).collect()
}

/// Does this directive open a block that `.ENDM` / `.ENDR` closes?
///
/// MACRO-11 terminates `.MACRO`, `.REPT`, `.IRP` and `.IRPC` alike with
/// `.ENDM`; `.ENDR` is the repeat-specific synonym. The corpus uses both
/// interchangeably — `CMR.MAC:111` and `CWV.MAC:743` close a `.REPT` with
/// `.ENDM`, while `HLL65F.MAC:224`'s `.IRP` closes with `.ENDR`. Treating them
/// as distinct made the scanner run to end-of-file, and since the failure path
/// skips everything it scanned, it silently swallowed the rest of six files —
/// taking every label defined after the block with it.
fn opens_block(d: Option<Dir>) -> bool {
    matches!(
        d,
        Some(Dir::Rept) | Some(Dir::Irp) | Some(Dir::Irpc) | Some(Dir::Macro)
    )
}

fn closes_block(d: Option<Dir>) -> bool {
    matches!(d, Some(Dir::Endr) | Some(Dir::Endm))
}

/// Split an operand field on commas that are not inside angle brackets.
fn split_commas<'t>(toks: Line<'t>) -> Vec<&'t [Token]> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::Punct('<') => depth += 1,
            Tok::Punct('>') => depth -= 1,
            Tok::Punct(',') if depth == 0 => {
                out.push(&toks[start..i]);
                start = i + 1;
            }
            Tok::Eol | Tok::Comment(_) => {
                out.push(&toks[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < toks.len() {
        out.push(&toks[start..]);
    }
    out.retain(|f| !f.is_empty());
    out
}

/// Render a text-condition argument for comparison, dropping the surrounding
/// angle brackets. Compared as tokens rather than raw text so that whitespace
/// differences do not matter.
fn strip_angles(toks: Line) -> Vec<Tok> {
    toks.iter()
        .map(|t| t.tok.clone())
        .filter(|t| !matches!(t, Tok::Punct('<') | Tok::Punct('>') | Tok::Eol | Tok::Comment(_)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(files: &[(&str, &str)]) -> HashMap<String, String> {
        files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn asm(files: &[(&str, &str)]) -> Result<BTreeMap<u16, u8>, Vec<String>> {
        let p = provider(files);
        let mut a = Assembler::new(&p);
        a.assemble("MAIN.MAC")
    }

    /// Assemble and hand back both the image and the recorded IR.
    fn asm_ir(files: &[(&str, &str)]) -> (BTreeMap<u16, u8>, crate::ir::Ir) {
        let p = provider(files);
        let mut a = Assembler::new(&p);
        let img = a.assemble("MAIN.MAC").expect("assembly failed");
        (img, a.ir.clone())
    }

    /// `.IRP` repeats once per comma-separated item. This is the shape Crystal
    /// Castles uses (`HLL65F.MAC:224`, `.IRP OPC,<ASL,ROL,...>`), which is why
    /// the bug below stayed hidden.
    #[test]
    fn irp_repeats_once_per_item() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.IRP OPC,<7,8,9>\n\
             	.BYTE OPC\n\
             	.ENDR\n\
             	.END\n",
        )])
        .expect("assembly failed");
        let bytes: Vec<u8> = (0xA000u16..0xA004).filter_map(|a| img.get(&a).copied()).collect();
        assert_eq!(bytes, vec![7, 8, 9]);
    }

    /// A parameter named after an addressing mode still works.
    ///
    /// `X,` is lexed as an indexed-addressing prefix that swallows the comma,
    /// so the name and the item list arrive in one field rather than two.
    /// Before this was recovered, the whole directive expanded to **nothing**,
    /// silently — no error, no bytes. Crystal Castles names its one `.IRP`
    /// parameter `OPC` and so never hit it.
    #[test]
    fn irp_accepts_a_parameter_named_after_an_addressing_mode() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.IRP X,<7,8,9>\n\
             	.BYTE X\n\
             	.ENDR\n\
             	.END\n",
        )])
        .expect("assembly failed");
        let bytes: Vec<u8> = (0xA000u16..0xA004).filter_map(|a| img.get(&a).copied()).collect();
        assert_eq!(bytes, vec![7, 8, 9], "the body must run, not be skipped");
    }

    /// `.IRPC` repeats once per character, and the substitution really varies:
    /// the three symbols it defines are referenced afterwards, so a body that
    /// silently expanded to nothing would fail as undefined rather than pass.
    #[test]
    fn irpc_repeats_once_per_character_and_concatenates() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.IRPC X,<012>\n\
             SYM'X=1\n\
             	.ENDR\n\
             	.BYTE SYM0,SYM1,SYM2\n\
             	.END\n",
        )])
        .expect("assembly failed");
        let bytes: Vec<u8> = (0xA000u16..0xA004).filter_map(|a| img.get(&a).copied()).collect();
        assert_eq!(bytes, vec![1, 1, 1], "SYM0, SYM1 and SYM2 must all exist");
    }

    #[test]
    fn irpc_iterates_a_blank_and_the_join_stops_at_it() {
        // Two rules meet on one line, and the corpus needs both. `VGMC.MAC`'s
        // `ALPHA` is `.IRPC ...X,<STRING>` around `JSRL CHAR.'...X`, and
        // `AS2ROM.MAC:1097` passes `<MCMLXXX ATARI IN>`:
        //
        //  - the blank is a character of the string, so it gets its own
        //    iteration — drop it and the list comes out two words short;
        //  - a blank cannot extend a symbol name, so `CHAR.` joined to a blank
        //    names `CHAR.`, the blank glyph at `VGAN.MAC:129`.
        //
        // `A B` is three characters, so three bytes, and the middle one must
        // come from the base symbol rather than from anything named "SYM ".
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             SYM=7\n\
             SYMA=1\n\
             SYMB=2\n\
             	.IRPC X,<A B>\n\
             	.BYTE SYM'X\n\
             	.ENDR\n\
             	.END\n",
        )])
        .expect("assembly failed");
        let bytes: Vec<u8> = (0xA000u16..0xA003).filter_map(|a| img.get(&a).copied()).collect();
        assert_eq!(
            bytes,
            vec![1, 7, 2],
            "the blank iterates, and joining onto it yields the base symbol"
        );
    }

    /// The characters come from the argument's raw text, not its value.
    ///
    /// `<0123>` lexes as a single number. Splitting what it evaluates to would
    /// give one iteration; splitting its digits gives four — which is what
    /// Space Duel's `ROCK'X'1` over `<0123>` relies on.
    #[test]
    fn irpc_splits_the_raw_text_not_the_evaluated_number() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.IRPC X,<0123>\n\
             	.BYTE 0FF\n\
             	.ENDR\n\
             	.END\n",
        )])
        .expect("assembly failed");
        let bytes: Vec<u8> = (0xA000u16..0xA010).filter_map(|a| img.get(&a).copied()).collect();
        assert_eq!(bytes, vec![0xFF; 4], "one iteration per digit");
    }

    /// `.IRPC` opens a block, so an unterminated one must be diagnosed rather
    /// than swallowing the rest of the file — the failure `opens_block` already
    /// carries a comment about.
    #[test]
    fn an_unterminated_irpc_is_reported() {
        let errs = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.IRPC X,<01>\n\
             	.BYTE 0FF\n\
             	.END\n",
        )])
        .expect_err("should not assemble");
        assert!(
            errs.iter().any(|e| e.contains(".IRPC without matching .ENDR")),
            "{errs:?}"
        );
    }

    #[test]
    fn nchr_counts_the_characters_of_its_argument() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.NCHR CNT,<HELLO>\n\
             	.BYTE CNT\n\
             	.END\n",
        )])
        .expect("assembly failed");
        assert_eq!(img.get(&0xA000), Some(&5));
    }

    /// The count is over raw characters, not an evaluated value: `<0123>` lexes
    /// as one number, and counting what it evaluates to would give 1.
    #[test]
    fn nchr_counts_raw_characters_not_an_evaluated_value() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.NCHR CNT,<0123>\n\
             	.BYTE CNT\n\
             	.END\n",
        )])
        .expect("assembly failed");
        assert_eq!(img.get(&0xA000), Some(&4));
    }

    /// `'X` is MACRO-11's character-value operator.
    #[test]
    fn a_quote_yields_the_character_code() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             V='A\n\
             	.BYTE V\n\
             	.END\n",
        )])
        .expect("assembly failed");
        assert_eq!(img.get(&0xA000), Some(&0x41));
    }

    /// The operator and the concatenation mark, spelled the same and adjacent.
    ///
    /// This is Space Duel's `ASCIN` shape: walk a string with `.IRPC` and take
    /// each character's code with `''PARAM` — the first quote is the operator,
    /// the second introduces the parameter.
    #[test]
    fn a_doubled_quote_is_the_operator_then_a_concatenation_mark() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.MACRO T STRING\n\
             	.IRPC C,<STRING>\n\
             	.BYTE ''C\n\
             	.ENDR\n\
             	.ENDM\n\
             	.=0A000\n\
             	T <AB>\n\
             	.END\n",
        )])
        .expect("assembly failed");
        let bytes: Vec<u8> = (0xA000u16..0xA002).filter_map(|a| img.get(&a).copied()).collect();
        assert_eq!(bytes, vec![0x41, 0x42]);
    }

    /// And with no space either, which is how the game actually writes it:
    /// `...4=''...5`. Space alone cannot distinguish operator from mark.
    #[test]
    fn the_operator_is_recognised_without_a_preceding_space() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.MACRO T STRING\n\
             	.IRPC C,<STRING>\n\
             V=''C\n\
             	.BYTE V\n\
             	.ENDR\n\
             	.ENDM\n\
             	.=0A000\n\
             	T <AB>\n\
             	.END\n",
        )])
        .expect("assembly failed");
        let bytes: Vec<u8> = (0xA000u16..0xA002).filter_map(|a| img.get(&a).copied()).collect();
        assert_eq!(bytes, vec![0x41, 0x42]);
    }

    /// Concatenation must still fuse. This is HLL65F's `B'COND` shape, which
    /// Crystal Castles depends on for its counted-shift macros.
    #[test]
    fn a_quote_between_text_and_a_parameter_still_concatenates() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.MACRO T SUF\n\
             	.BYTE VAL'SUF\n\
             	.ENDM\n\
             VALX=07\n\
             	.=0A000\n\
             	T X\n\
             	.END\n",
        )])
        .expect("assembly failed");
        assert_eq!(img.get(&0xA000), Some(&7), "VAL'SUF must fuse to VALX");
    }

    #[test]
    fn the_ir_re_encodes_to_the_image_it_recorded() {
        // The assertion that the IR *is* the program rather than a plausible
        // story about it.
        let (img, ir) = asm_ir(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             START:	LDA I,012\n\
             	STA 0200\n\
             	LDX 0FF\n\
             LOOP:	DEX\n\
             	BNE LOOP\n\
             	RTS\n",
        )]);
        assert_eq!(ir.check_against(&img), Vec::<String>::new());
        assert_eq!(ir.instructions().count(), 6);
    }

    #[test]
    fn the_ir_records_addresses_sizes_and_operand_text() {
        let (_, ir) = asm_ir(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	LDA I,012\n\
             	STA 0200\n",
        )]);
        let i: Vec<_> = ir.instructions().collect();
        assert_eq!(i[0].addr, 0xA000);
        assert_eq!(i[0].mnemonic, "LDA");
        assert_eq!(i[0].size, 2);
        assert_eq!(i[1].addr, 0xA002);
        assert_eq!(i[1].size, 3);
        assert!(i[1].operand_text.contains("0200"), "{:?}", i[1].operand_text);
    }

    #[test]
    fn the_ir_separates_data_runs_from_instructions() {
        let (_, ir) = asm_ir(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	LDA I,01\n\
             TABLE:	.BYTE 1,2,3,4\n\
             	RTS\n",
        )]);
        let data: Vec<_> = ir.data().collect();
        assert_eq!(data.len(), 1, "the four bytes coalesce into one run");
        assert_eq!(data[0].addr, 0xA002);
        assert_eq!(data[0].len, 4);
        assert_eq!(ir.instructions().count(), 2, "data is not an instruction");
    }

    #[test]
    fn the_ir_records_label_scope_as_the_source_declares_it() {
        let (_, ir) = asm_ir(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             PRIV:	RTS\n\
             SHARED::	RTS\n",
        )]);
        let by = |n: &str| ir.labels().find(|l| l.name == n).cloned();
        assert_eq!(by("PRIV").expect("PRIV").scope, crate::ir::Scope::Unit);
        assert_eq!(by("SHARED").expect("SHARED").scope, crate::ir::Scope::Global);
    }

    #[test]
    fn the_ir_carries_the_macro_chain_that_emitted_each_instruction() {
        let (_, ir) = asm_ir(&[(
            "MAIN.MAC",
            "	.MACRO WRAP\n\
             	LDA I,07\n\
             	.ENDM\n\
             	.=0A000\n\
             	WRAP\n\
             	RTS\n",
        )]);
        let i: Vec<_> = ir.instructions().collect();
        assert_eq!(i[0].chain, vec!["WRAP".to_string()], "expanded from WRAP");
        assert!(i[1].chain.is_empty(), "the RTS was written directly");
        assert!(i[0].expanded_from(&["WRAP"]));
        assert!(!i[1].expanded_from(&["WRAP"]));
    }

    fn bytes(img: &BTreeMap<u16, u8>, from: u16, n: usize) -> Vec<u8> {
        (0..n).map(|i| *img.get(&(from + i as u16)).unwrap_or(&0)).collect()
    }

    #[test]
    fn location_counter_set_and_backed_up() {
        // The LDAL idiom from CRP.MAC: emit an opcode byte and a .WORD, then
        // back the counter up one so the next emission overwrites the word's
        // high byte.
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.BYTE 0A9\n\
             	.WORD 1234\n\
             	.=.-1\n\
             	.BYTE 0EA\n",
        )])
        .expect("assembly failed");
        // A9 34 EA — the 0x12 written by .WORD is overwritten in place.
        assert_eq!(bytes(&img, 0xA000, 3), vec![0xA9, 0x34, 0xEA]);
        assert_eq!(img.len(), 3, "no stray bytes");
    }

    #[test]
    fn rept_zero_skips_prose_without_parsing_it() {
        // CCN.MAC wraps documentation in .REPT 0. The prose is not valid syntax
        // and must never be assembled.
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.BYTE 1\n\
             	.REPT 0\n\
             $COINA:   \"MECHS\" locations containing coin switches in D7. Left mech is at  $COINA,  Right\n\
             	See bit definitions in DEFAULT ASSIGNMENTS, and Macro definitions GCM,GHM,GDM.\n\
             	.ENDR\n\
             	.BYTE 2\n",
        )])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 2), vec![1, 2]);
        assert_eq!(img.len(), 2, "prose leaked into the image");
    }

    #[test]
    fn rept_repeats_and_nests() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n	.REPT 3\n	.BYTE 0FF\n	.ENDR\n",
        )])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 3), vec![0xFF, 0xFF, 0xFF]);
        assert_eq!(img.len(), 3);

        // C99.MAC pads each castle block this way: .REPT n*WV.SIZ+WV.STR-.
        let img = asm(&[(
            "MAIN.MAC",
            "WV.STR=0A000\nWV.SIZ=4\n	.=WV.STR\n	.BYTE 1\n	.REPT 1*WV.SIZ+WV.STR-.\n	.BYTE 0FF\n	.ENDR\n",
        )])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 4), vec![1, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn radix_changes_and_restores() {
        // C99.MAC switches to .RADIX 10 around each .DAT include and back to 16.
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.BYTE 10\n\
             	.RADIX 10\n\
             	.BYTE 10\n\
             	.RADIX 16\n\
             	.BYTE 10\n",
        )])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 3), vec![0x10, 10, 0x10]);
    }

    #[test]
    fn radix_operand_is_always_decimal() {
        // .RADIX 10 under hex must mean ten, not sixteen.
        let img = asm(&[("MAIN.MAC", "	.=0A000\n	.RADIX 10\n	.BYTE 99\n")])
            .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 1), vec![99]);
    }

    #[test]
    fn error_fires_only_in_a_taken_branch() {
        // CG.MAC: .IF GE,.-0F0 / .ERROR ; RAM overlap with sounds / .ENDC
        //
        // `.ERROR` reports but does not abort: `CRF.MAC:72` carries an
        // unconditional `.error ; dummy` in shipped source, so a fatal reading
        // would make the real program unbuildable. The guard must therefore be
        // observed in the warnings, not the errors.
        let fired = provider(&[(
            "MAIN.MAC",
            "	.=0200\n	.IF GE,.-0F0\n	.ERROR ;  RAM overlap with sounds\n	.ENDC\n",
        )]);
        let mut a = Assembler::new(&fired);
        a.assemble("MAIN.MAC").expect("must not abort");
        assert!(
            a.warnings.iter().any(|w| w.contains("RAM overlap")),
            "guard should have reported: {:?}",
            a.warnings
        );

        // Counter below the limit: .-0F0 is negative, so the guard stays quiet.
        let quiet = provider(&[(
            "MAIN.MAC",
            "	.=00E0\n	.IF GE,.-0F0\n	.ERROR ;  RAM overlap with sounds\n	.ENDC\n",
        )]);
        let mut a = Assembler::new(&quiet);
        a.assemble("MAIN.MAC").unwrap();
        assert!(
            a.warnings.is_empty(),
            "guard fired backwards: {:?}",
            a.warnings
        );
    }

    #[test]
    fn include_resolves_with_and_without_extension() {
        // CSTART.MAC writes `.INCLUDE HLL65F`; others write `.INCLUDE CMAC.MAC`.
        let img = asm(&[
            ("MAIN.MAC", "	.=0A000\n	.INCLUDE HLL65F\n	.INCLUDE SUB.MAC\n"),
            ("HLL65F.MAC", "	.BYTE 1\n"),
            ("SUB.MAC", "	.BYTE 2\n"),
        ])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 2), vec![1, 2]);
    }

    #[test]
    fn location_counter_crosses_include_boundaries() {
        // Region context crosses files — the point writescan.py had to learn.
        let img = asm(&[
            ("MAIN.MAC", "	.=0A000\n	.BYTE 1\n	.INCLUDE SUB\n	.BYTE 3\n"),
            ("SUB.MAC", "	.BYTE 2\n"),
        ])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 3), vec![1, 2, 3]);
    }

    #[test]
    fn m68_reverses_word_order_until_disabled() {
        // CRP.MAC's LDAH: .BYTE 0A9 / .ENABL M68 / .WORD ...1 / .DSABL M68 / .=.-1
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.WORD 1234\n\
             	.ENABL	M68\n\
             	.WORD 1234\n\
             	.DSABL	M68\n\
             	.WORD 1234\n",
        )])
        .expect("assembly failed");
        assert_eq!(
            bytes(&img, 0xA000, 6),
            vec![0x34, 0x12, 0x12, 0x34, 0x34, 0x12]
        );
    }

    #[test]
    fn ldah_idiom_end_to_end() {
        // The whole CRP.MAC LDAH macro body, hand-expanded: it must leave the
        // high byte of the symbol as an immediate operand.
        let img = asm(&[(
            "MAIN.MAC",
            "STRET=0C123\n	.=0A000\n	.BYTE 0A9\n	.ENABL M68\n	.WORD STRET-1\n	.DSABL M68\n	.=.-1\n	.BYTE 048\n",
        )])
        .expect("assembly failed");
        // STRET-1 is 0C122; under M68 the .WORD lays down C1 then 22, and the
        // .=.-1 leaves the counter on the 22 so the *next* emission overwrites
        // it. Result: LDA #0C1 (the high byte) followed by the next opcode.
        assert_eq!(bytes(&img, 0xA000, 3), vec![0xA9, 0xC1, 0x48]);
        assert_eq!(img.len(), 3);
    }

    #[test]
    fn forward_references_resolve_on_the_second_pass() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n	.WORD LATER\n LATER:	.BYTE 0EE\n",
        )])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 3), vec![0x02, 0xA0, 0xEE]);
    }

    #[test]
    fn conditionals_nest_and_dead_branches_stay_dead() {
        let img = asm(&[(
            "MAIN.MAC",
            "FLAG=0\n	.=0A000\n\
             	.IF NE,FLAG\n\
             	.BYTE 0AA\n\
             	.IF EQ,FLAG\n	.BYTE 0BB\n	.ENDC\n\
             	.IFF\n	.BYTE 0CC\n\
             	.ENDC\n",
        )])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 1), vec![0xCC]);
        assert_eq!(img.len(), 1);
    }

    #[test]
    fn subconditionals_refer_to_the_original_test_not_a_chain() {
        // `.IFT` / `.IFF` / `.IFTF` are subconditionals on the enclosing `.IF`'s
        // result — "if true", "if false", "if true or false" — and may repeat
        // within one frame. CCN.MAC does exactly that: `.IF EQ,MECHS-1` is
        // followed by .IFF, .IFTF, .IFT, .IFF, .IFTF, all referring back to the
        // same test.
        //
        // Modelling them as an if/elseif chain makes the *second* .IFT activate
        // on the strength of the earlier .IFF having fired — which assembles
        // BOTH arms of a two-way choice and quietly produces wrong code.
        let img = asm(&[(
            "MAIN.MAC",
            "FLAG=0\n	.=0A000\n\
             	.IF NE,FLAG\n	.BYTE 0AA\n\
             	.IFF\n	.BYTE 0BB\n\
             	.IFTF\n	.BYTE 0CC\n\
             	.IFT\n	.BYTE 0DD\n\
             	.IFF\n	.BYTE 0EE\n\
             	.IFTF\n	.BYTE 0FF\n\
             	.ENDC\n",
        )])
        .expect("assembly failed");
        // FLAG is 0, so NE is false: the .IFT arms stay dead throughout.
        assert_eq!(bytes(&img, 0xA000, 4), vec![0xBB, 0xCC, 0xEE, 0xFF]);
        assert_eq!(img.len(), 4, "an .IFT arm assembled on a false condition");

        // And the mirror image, so the fix is not simply inverted.
        let img = asm(&[(
            "MAIN.MAC",
            "FLAG=1\n	.=0A000\n\
             	.IF NE,FLAG\n	.BYTE 0AA\n\
             	.IFF\n	.BYTE 0BB\n\
             	.IFTF\n	.BYTE 0CC\n\
             	.IFT\n	.BYTE 0DD\n\
             	.IFF\n	.BYTE 0EE\n\
             	.ENDC\n",
        )])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 3), vec![0xAA, 0xCC, 0xDD]);
        assert_eq!(img.len(), 3);
    }

    #[test]
    fn globl_declared_externals_size_absolute() {
        // A symbol declared `.GLOBL` but not defined in this unit is an import:
        // the assembler cannot know it fits the zero page, so the operand is
        // absolute even though the value is small. CRP.MAC:52 declares
        // `.GLOBL ...,SN.NUM` and the oracle assembles `STA SN.NUM` (00BA) as
        // `8d ba 00`; the mis-typed `.GLOBB $INTCT,ATRACT` two lines later never
        // declared those, and the oracle assembles them zero page — `a5 a3`.
        // The misspelling changed instruction sizes.
        let p = provider(&[
            ("A.MAC", "	.=0090\nZP::	.BLKB 1\n"),
            (
                "B.MAC",
                "	.ENABL AMA\n	.GLOBL ZP\n	.=0A000\n	STA ZP\n",
            ),
            (
                "C.MAC",
                "	.ENABL AMA\n	.=0B000\n	STA ZP\n",
            ),
        ]);
        let mut a = Assembler::new(&p);
        let img = a
            .assemble_units(&["A.MAC", "B.MAC", "C.MAC"])
            .expect("assembly failed");

        assert_eq!(a.globals.get("ZP"), Some(&0x0090));
        // B declares it `.GLOBL` without defining it: an import, sized absolute.
        assert_eq!(
            bytes(&img, 0xA000, 3),
            vec![0x8D, 0x90, 0x00],
            "declared external must size absolute"
        );
        // C does not declare it, so it sizes on the value like any other symbol.
        assert_eq!(
            bytes(&img, 0xB000, 2),
            vec![0x85, 0x90],
            "undeclared reference sizes zero page"
        );
    }

    #[test]
    fn ndf_and_text_conditions() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             	.IIF NDF,INCLUDE,INCLUDE=1\n\
             	.IF NDF,NOSUCH\n	.BYTE 1\n	.ENDC\n\
             	.IF NB,<SOMETHING>\n	.BYTE 2\n	.ENDC\n\
             	.IF B,<>\n	.BYTE 3\n	.ENDC\n\
             	.IF IDN,<ABC>,<ABC>\n	.BYTE 4\n	.ENDC\n\
             	.IF DIF,<ABC>,<XYZ>\n	.BYTE 5\n	.ENDC\n",
        )])
        .expect("assembly failed");
        assert_eq!(bytes(&img, 0xA000, 5), vec![1, 2, 3, 4, 5]);
        // The .IIF defined INCLUDE, so it is no longer undefined. Note the
        // symbol is stored as INCLUD: MACRO-11 truncates to six RAD50
        // characters, and `INCLUDE` is seven. Definition and reference truncate
        // alike, so `CG.MAC:340`'s `INCLUDE=1` and CCN.MAC's `.IIF NDF,INCLUDE`
        // still meet.
        let p = provider(&[("MAIN.MAC", "	.IIF NDF,INCLUDE,INCLUDE=1\n")]);
        let mut a = Assembler::new(&p);
        a.assemble("MAIN.MAC").unwrap();
        assert_eq!(a.lookup("INCLUD"), Some(1));
        // Both spellings resolve, because lookup truncates exactly as
        // definition does — that is the whole point.
        assert_eq!(a.lookup("INCLUDE"), Some(1));
        assert!(
            a.locals.contains_key("INCLUD") && !a.locals.contains_key("INCLUDE"),
            "stored under the truncated key"
        );
    }

    #[test]
    fn blkb_and_blkw_reserve_without_emitting() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n	.BYTE 1\n	.BLKB 3\n	.BYTE 2\n	.BLKW 2\n	.BYTE 3\n",
        )])
        .expect("assembly failed");
        assert_eq!(img.get(&0xA000), Some(&1));
        assert_eq!(img.get(&0xA004), Some(&2));
        assert_eq!(img.get(&0xA009), Some(&3));
        assert_eq!(img.len(), 3, "reserved space must not be emitted");
    }

    #[test]
    fn listing_directives_parse_and_emit_nothing() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.TITLE CLDAT\n	.ASECT\n	.ENABLE AMA\n	.LIST ME\n	.SBTTL RPM CONFIGURATION\n	.PAGE\n	.=0A000\n	.BYTE 7\n",
        )])
        .expect("assembly failed");
        assert_eq!(img.len(), 1);
        assert_eq!(img.get(&0xA000), Some(&7));
    }

    /// Assemble a real declaration file end to end. `CG.MAC` is almost entirely
    /// equates, `.BLKB` reservations and `.IF GE` guards, so it exercises the
    /// location counter, include handling and signed conditionals against
    /// genuine input rather than fixtures.
    ///
    /// Its two guards must stay *silent*: firing them would mean the RAM layout
    /// had overflowed, which it demonstrably does not in shipped source.
    ///
    /// ```text
    /// CHILL65_CORPUS=/path/to/crystal-castles cargo test -p chill65-asm -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
    fn assembles_real_declaration_file() {
        let Ok(dir) = std::env::var("CHILL65_CORPUS") else {
            eprintln!("CHILL65_CORPUS unset — skipping");
            return;
        };
        let p = DirProvider {
            root: std::path::PathBuf::from(&dir),
        };
        let mut a = Assembler::new(&p);
        let errs = a.assemble("CG.MAC").err().unwrap_or_default();

        eprintln!(
            "CG.MAC: {} symbols, {} unhandled lines, {} errors",
            (a.locals.len() + a.globals.len()),
            a.unhandled,
            errs.len()
        );
        for e in &errs {
            eprintln!("    {e}");
        }

        // One known dialect gap remains, deliberately not guessed at: the
        // checksum equates use a `?` construct
        //
        //     CHK01 = 082?7F?01      (root)
        //     CHK01 = 73?1           (version-2)
        //     CHK01 = 0E7            (version-3, no `?` at all)
        //
        // whose meaning is unresolved. These bytes are emitted into the ROM
        // (`CGR.MAC:5 .BYTE CHK01`, `CCUBE.MAC:208 CHK02`, `CIN.MAC:179 CHK03`)
        // so getting them right matters for the task 13 gate, but XOR — which
        // reproduces the root tree's ROM byte at A024 exactly (82^7F^01 = FC)
        // — does *not* fit version-2 or version-3. Left for task 12 rather than
        // implemented on a one-in-three match. See dialect.md.
        assert!(
            errs.iter().all(|e| e.contains("Punct('?')")),
            "unexpected errors beyond the known `?` gap: {errs:#?}"
        );

        // Everything task 9 owns must work on real input.
        assert_eq!(a.lookup("WV.STR"), Some(0xA000), "WV.STR");
        assert_eq!(a.lookup("RAM.ST"), Some(0x8000), "RAM.ST");
        assert_eq!(a.lookup("WV.SIZ"), Some(0x0400), "WV.SIZ");
        assert_eq!(a.lookup("EEROM"), Some(0x9000), "EEROM (via CEEDEF)");
        assert_eq!(a.lookup("HW.BSL"), Some(0x9E87), "bank select latch");

        // PL.AZ = .-PL.1A — a location-counter difference over a run of .BLKBs.
        let pl_az = a.lookup("PL.AZ").expect("PL.AZ not defined");
        assert!(pl_az > 0 && pl_az < 0x1000, "PL.AZ implausible: {pl_az:#x}");

        // The two RAM-overflow guards must NOT have fired: shipped source does
        // not overflow, so a firing guard would mean the signed comparison or
        // the location counter is wrong.
        assert!(
            !errs.iter().any(|e| e.contains(".ERROR")),
            "a RAM-overflow guard fired on shipped source: {errs:#?}"
        );
    }

    #[test]
    fn macro_defines_and_expands() {
        let img = asm(&[(
            "MAIN.MAC",
            "	.MACRO TRAI FROM,TO\n	LDA I,FROM\n	STA TO\n	.ENDM\n\
             	.=0E000\n	TRAI 0 09E87\n",
        )])
        .expect("assembly failed");
        // LDA #0 / STA 9E87 — the real bytes at E000 in the ROM image.
        assert_eq!(bytes(&img, 0xE000, 5), vec![0xA9, 0x00, 0x8D, 0x87, 0x9E]);
    }

    #[test]
    fn nested_macro_expansion_reaches_leaf_instructions() {
        // M6502.MAC: TR24AI calls TRAI and TR16AI.
        let img = asm(&[(
            "MAIN.MAC",
            "	.MACRO TRAI FROM,TO\n	LDA I,FROM\n	STA TO\n	.ENDM\n\
             	.MACRO TR16AI FROM,TO\n	LDA I,FROM&^H0FF\n	STA TO\n\
             	LDA I,FROM&^H0FF00/^H100\n	STA 1+TO\n	.ENDM\n\
             	.MACRO TR24AI FROM1,FROM2,TO\n	TRAI FROM1 2+TO\n	TR16AI FROM2 TO\n	.ENDM\n\
             	.=0A000\n	TR24AI 7 0BEEF 0800\n",
        )])
        .expect("assembly failed");
        assert_eq!(
            bytes(&img, 0xA000, 12),
            vec![
                0xA9, 0x07, 0x8D, 0x02, 0x08, // LDA #7  / STA 0802
                0xA9, 0xEF, 0x8D, 0x00, 0x08, // LDA #EF / STA 0800
                0xA9, 0xBE, // LDA #BE
            ]
        );
    }

    #[test]
    fn runaway_recursion_is_caught_and_names_the_chain() {
        let errs = asm(&[(
            "MAIN.MAC",
            "	.MACRO LOOPY\n	LOOPY\n	.ENDM\n	.=0A000\n	LOOPY\n",
        )])
        .expect_err("runaway recursion should fail");
        assert!(
            errs.iter().any(|e| e.contains("too deep") && e.contains("LOOPY")),
            "error must name the chain: {errs:#?}"
        );
    }

    #[test]
    fn a_macro_can_define_a_macro() {
        // HLL65F's DEFIF is exactly this shape, and it is how IFEQ/IFNE/PLEND
        // and the rest come into existence.
        let img = asm(&[(
            "MAIN.MAC",
            "	.MACRO DEFIF .1.,.2.\n  .MACRO .1.\n	.2.	.\n	.ENDM\n	.ENDM\n\
             	DEFIF IFEQ,BNE\n\
             	.=0A000\n	IFEQ\n",
        )])
        .expect("assembly failed");
        // DEFIF IFEQ,BNE defines IFEQ to emit BNE — the INVERSE condition,
        // because the branch jumps over the block.
        assert_eq!(img.get(&0xA000), Some(&0xD0), "IFEQ must emit BNE (0xD0)");
    }

    #[test]
    fn defstack_push_and_pop_round_trip() {
        // HLL65F saves and restores its scratch symbols this way, popping in
        // reverse order to restore them.
        let p = provider(&[(
            "MAIN.MAC",
            // Use HLL65F's own scratch-symbol names. Single letters would not
             // do: `A,` lexes as an addressing-mode PREFIX, since A is one of
             // the fourteen mode names. No corpus symbol collides that way
             // (checked across all three trees) but a test easily could.
             "	.DEFSTACK REGSAV,4.\n...P0=1\n...S0=2\n\
             	.PUSH REGSAV,...P0,...S0\n...P0=99\n...S0=98\n	.POP REGSAV,...S0,...P0\n",
        )]);
        let mut a = Assembler::new(&p);
        a.assemble("MAIN.MAC").unwrap();
        assert_eq!(a.lookup("...P0"), Some(1), "...P0 not restored");
        assert_eq!(a.lookup("...S0"), Some(2), "...S0 not restored");
    }

    #[test]
    fn location_rewind_patches_a_branch_operand() {
        // The FND mechanism in miniature: emit a branch with a placeholder
        // operand, then rewind the location counter and write the real offset
        // over it. This is what produces the overlapping writes in CRF.LDA.
        let img = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n\
             P1=.\n	BNE .\n\
             	.BYTE 0EA\n	.BYTE 0EA\n\
             P0=.\n\
             	.=P1+1\n	.BYTE P0-P1-2\n	.=P0\n",
        )])
        .expect("assembly failed");
        // BNE +2 skips the two NOPs.
        assert_eq!(bytes(&img, 0xA000, 4), vec![0xD0, 0x02, 0xEA, 0xEA]);
        assert_eq!(img.len(), 4, "the placeholder must be overwritten, not appended");
    }

    #[test]
    fn branch_out_of_range_is_reported() {
        let errs = asm(&[(
            "MAIN.MAC",
            "	.=0A000\n	BNE FAR\n	.BLKB 200\nFAR:	.BYTE 0\n",
        )])
        .expect_err("a branch 200 bytes away must not assemble");
        assert!(
            errs.iter().any(|e| e.contains("out of range")),
            "expected a range error: {errs:#?}"
        );
    }

    #[test]
    fn ama_sizing_is_consistent_across_passes() {
        // A forward reference is unresolved in pass one, so the mode is chosen
        // as absolute; pass two must not change its mind and shorten it.
        let img = asm(&[(
            "MAIN.MAC",
            "	.ENABL AMA\n	.=0A000\n	LDA LATER\n	.BYTE 0EA\nLATER=004\n",
        )])
        .expect("assembly failed");
        assert_eq!(
            bytes(&img, 0xA000, 4),
            vec![0xAD, 0x04, 0x00, 0xEA],
            "forward-referenced operand must stay absolute in both passes"
        );
    }

    /// Drive the *real* `HLL65F.MAC` rather than a reimplementation.
    ///
    /// The structured-control-flow package is 4.5 KB of third-party source and
    /// is not copied into this repository, so this test is ignored by default
    /// and runs against a local corpus:
    ///
    /// ```text
    /// CHILL65_CORPUS=/path/to/crystal-castles cargo test -p chill65-asm -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
    fn hll65f_structured_control_flow() {
        let Ok(dir) = std::env::var("CHILL65_CORPUS") else {
            eprintln!("CHILL65_CORPUS unset — skipping");
            return;
        };
        let root = std::path::PathBuf::from(&dir);

        // Serve the driver from memory and everything else from the corpus, so
        // nothing is written into the third-party tree (which would also race
        // the lexer smoke test running in parallel).
        struct Overlay {
            root: std::path::PathBuf,
            driver: &'static str,
        }
        impl SourceProvider for Overlay {
            fn load(&self, name: &str) -> Option<Vec<u8>> {
                if name == "DRIVER" {
                    return Some(self.driver.as_bytes().to_vec());
                }
                DirProvider {
                    root: self.root.clone(),
                }
                .load(name)
            }
        }

        let p = Overlay {
            root: root.clone(),
            driver: "\t.INCLUDE HLL65F\n\t.=0A000\nSTART:\n\tLDA I,0\n\tIFEQ\n\tNOP\n\tENDIF\n\tRTS\n",
        };
        let mut a = Assembler::new(&p);
        let result = a.assemble("DRIVER");

        match result {
            Ok(img) => {
                let b: Vec<u8> = (0..8)
                    .map(|i| *img.get(&(0xA000 + i)).unwrap_or(&0))
                    .collect();
                eprintln!("HLL65F IFEQ/ENDIF -> {b:02x?}");
                // LDA #0 ; BNE +1 ; NOP ; RTS
                assert_eq!(b[0], 0xA9, "LDA immediate");
                assert_eq!(b[2], 0xD0, "IFEQ must emit BNE (inverse condition)");
                assert_eq!(b[3], 0x01, "branch must skip exactly the NOP");
                assert_eq!(b[4], 0xEA, "NOP");
                assert_eq!(b[5], 0x60, "RTS");
            }
            Err(errs) => panic!("HLL65F driver failed: {errs:#?}"),
        }
    }

    /// Run a driver against the real `HLL65F.MAC` from a local corpus.
    #[cfg(test)]
    fn hll65f_assemble(driver: String) -> Result<BTreeMap<u16, u8>, Vec<String>> {
        let dir = std::env::var("CHILL65_CORPUS").expect("CHILL65_CORPUS");
        struct Overlay {
            root: std::path::PathBuf,
            driver: String,
        }
        impl SourceProvider for Overlay {
            fn load(&self, name: &str) -> Option<Vec<u8>> {
                if name == "DRIVER" {
                    return Some(self.driver.as_bytes().to_vec());
                }
                DirProvider {
                    root: self.root.clone(),
                }
                .load(name)
            }
        }
        let p = Overlay {
            root: std::path::PathBuf::from(dir),
            driver,
        };
        Assembler::new(&p).assemble("DRIVER")
    }

    /// `BEGIN` … `PLEND` — a backward branch to the top of the loop.
    ///
    /// `BEGIN` is `LOC 2`; `PLEND` comes from `DEFEND PLEND,BMI,BPL`, so it
    /// emits `BMI loop_start` when the loop fits in a relative branch.
    /// The shape is `CEN.MAC`'s `EN.INI`.
    #[test]
    #[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
    fn hll65f_begin_plend_loop() {
        if std::env::var("CHILL65_CORPUS").is_err() {
            return;
        }
        let img = hll65f_assemble(
            "\t.INCLUDE HLL65F\n\t.=0A000\n\tBEGIN\n\tNOP\n\tPLEND\n".into(),
        )
        .expect("BEGIN/PLEND driver failed");
        let b: Vec<u8> = (0..3).map(|i| *img.get(&(0xA000 + i)).unwrap_or(&0)).collect();
        eprintln!("HLL65F BEGIN/PLEND -> {b:02x?}");
        assert_eq!(b[0], 0xEA, "NOP");
        assert_eq!(b[1], 0x30, "PLEND must emit BMI");
        // BMI sits at A001, so a branch back to A000 is -3.
        assert_eq!(b[2], 0xFD, "backward branch offset to the loop top");
    }

    /// `IFEQ` … `ELSE` … `ENDIF` — two patched branches.
    #[test]
    #[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
    fn hll65f_if_else_endif() {
        if std::env::var("CHILL65_CORPUS").is_err() {
            return;
        }
        let img = hll65f_assemble(
            "\t.INCLUDE HLL65F\n\t.=0A000\n\tLDA I,0\n\tIFEQ\n\tNOP\n\tELSE\n\tTAX\n\tENDIF\n\tRTS\n"
                .into(),
        )
        .expect("IF/ELSE/ENDIF driver failed");
        let b: Vec<u8> = (0..10).map(|i| *img.get(&(0xA000 + i)).unwrap_or(&0)).collect();
        eprintln!("HLL65F IF/ELSE/ENDIF -> {b:02x?}");
        // LDA #0 ; BNE over-the-then ; NOP ; JMP over-the-else ; TAX ; RTS
        assert_eq!(b[0], 0xA9);
        assert_eq!(b[2], 0xD0, "IFEQ emits BNE");
        assert_eq!(b[4], 0xEA, "then-branch NOP");
        assert_eq!(b[5], 0x4C, "ELSE emits an absolute JMP past the else-branch");
        assert_eq!(b[8], 0xAA, "else-branch TAX");
        assert_eq!(b[9], 0x60, "RTS");
        // The BNE must land on the TAX, i.e. skip NOP + the 3-byte JMP.
        assert_eq!(b[3], 0x04, "BNE must skip the then-branch and the JMP");
        // And the JMP must target the RTS.
        assert_eq!((b[6], b[7]), (0x09, 0xA0), "JMP target is the RTS at A009");
    }

    /// `FND`'s own range check fires when a block will not fit in a branch.
    #[test]
    #[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
    fn hll65f_branch_out_of_range_errors() {
        if std::env::var("CHILL65_CORPUS").is_err() {
            return;
        }
        // `.ERROR` reports rather than aborts, so the guard shows in warnings.
        let dir = std::env::var("CHILL65_CORPUS").expect("CHILL65_CORPUS");
        struct Ov {
            root: std::path::PathBuf,
            driver: String,
        }
        impl SourceProvider for Ov {
            fn load(&self, name: &str) -> Option<Vec<u8>> {
                if name == "DRIVER" {
                    return Some(self.driver.as_bytes().to_vec());
                }
                DirProvider { root: self.root.clone() }.load(name)
            }
        }
        let p = Ov {
            root: std::path::PathBuf::from(dir),
            driver:
                "\t.INCLUDE HLL65F\n\t.=0A000\n\tLDA I,0\n\tIFEQ\n\t.BLKB 200\n\tENDIF\n\tRTS\n"
                    .into(),
        };
        let mut a = Assembler::new(&p);
        let _ = a.assemble("DRIVER");
        eprintln!("HLL65F range warning -> {:#?}", a.warnings);
        assert!(
            a.warnings.iter().any(|w| w.to_uppercase().contains("BRANCH OUT OF RANGE")),
            "expected HLL65F's own BRANCH OUT OF RANGE .ERROR: {:#?}",
            a.warnings
        );
    }

    #[test]
    fn private_and_shared_symbols_do_not_collide() {
        // The rule the source itself declares: `NAME:` is private to its unit,
        // `NAME::` is shared. Two units may define the same name and mean
        // different things — which is exactly what CG.MAC and CRP.MAC do with
        // $INTCT and ATRACT.
        let p = provider(&[
            ("A.MAC", "	.=0100\nSHARED::	.BLKB 1\nPRIV:	.BLKB 1\n	.BYTE SHARED\n	.BYTE PRIV\n"),
            ("B.MAC", "	.=0200\nPRIV:	.BLKB 1\n	.BYTE SHARED\n	.BYTE PRIV\n"),
        ]);
        let mut a = Assembler::new(&p);
        let img = a.assemble_units(&["A.MAC", "B.MAC"]).expect("assembly failed");

        // The shared symbol is visible from both units and reads the same.
        assert_eq!(a.globals.get("SHARED"), Some(&0x0100));
        assert_eq!(img.get(&0x0102), Some(&0x00), "A sees SHARED");
        assert_eq!(img.get(&0x0201), Some(&0x00), "B sees SHARED too");

        // The private name resolves differently in each unit.
        let a_priv = a.unit_locals["A.MAC"]["PRIV"];
        let b_priv = a.unit_locals["B.MAC"]["PRIV"];
        assert_eq!(a_priv, 0x0101);
        assert_eq!(b_priv, 0x0200);
        assert_ne!(
            a_priv, b_priv,
            "two units' private symbols must not be the same variable"
        );
        assert_eq!(img.get(&0x0103), Some(&0x01), "A's PRIV is A's");
        assert_eq!(img.get(&0x0202), Some(&0x00), "B's PRIV is B's");
        // And a private name never leaks into the shared table.
        assert!(!a.globals.contains_key("PRIV"));
    }

    #[test]
    fn an_argument_wrapped_in_angle_brackets_is_unwrapped() {
        // `ASTRD2.MAC:3684` passes indexed operands to a macro:
        // `DNEGATE <X,YINCL-ZSHIP>,<X,YINC>`, whose body does `SBC AA`.
        // MACRO-11 spells expression grouping with the same brackets, so
        // keeping them made the body read `SBC <X,YINCL-ZSHIP>` and the
        // evaluator met the index prefix where it wanted an operand — the
        // macro emitted nothing, and every byte after it in the module shifted.
        let src = "	.ENABL AMA\n	.RADIX 16\nYINCL = 040\nZSHIP = 014\n	.=0100\n\
                   	.MACRO DNEGATE,AA,BB\n	SBC AA\n	STA BB\n	.ENDM\n\
                   	DNEGATE <X,YINCL-ZSHIP>,<X,YINCL>\n";
        let p = provider(&[("A.MAC", src)]);
        let mut a = Assembler::new(&p);
        let img = a.assemble("A.MAC").expect("assembly failed");
        // SBC 2C,X then STA 40,X — both indexed, both zero page under AMA.
        assert_eq!(bytes(&img, 0x0100, 4), vec![0xF5, 0x2C, 0x95, 0x40]);
    }

    #[test]
    fn a_globl_after_the_definition_still_exports_it() {
        // Whether a name is declared global is a fact about the unit, not about
        // how far pass one has read. `AS2DEC.MAC`'s HEAD macro writes the
        // assignment first and the declaration two lines after it:
        //
        // ```text
        // .MACRO HEAD NAME
        // NAME'8 = ...
        // NAME'9 = ...
        // .GLOBL NAME'8
        // .GLOBL NAME'9
        // ```
        //
        // Consulting the declaration set at the moment of definition therefore
        // exported nothing HEAD produced, and every `BOX18`/`WNDSE9` in the
        // build came up undefined.
        let p = provider(&[
            (
                "A.MAC",
                "	.MACRO HEAD NAME\nNAME'8 = 05\n	.GLOBL NAME'8\n	.ENDM\n\
                 	HEAD BOX1\n	HEAD WNDSE\n",
            ),
            ("B.MAC", "	.=0A000\n	.BYTE BOX18\n	.BYTE WNDSE8\n"),
        ]);
        let mut a = Assembler::new(&p);
        let img = a.assemble_units(&["A.MAC", "B.MAC"]).expect("assembly failed");

        assert_eq!(a.globals.get("BOX18"), Some(&5), "declared after definition");
        assert_eq!(a.globals.get("WNDSE8"), Some(&5));
        assert_eq!(bytes(&img, 0xA000, 2), vec![0x05, 0x05], "another unit sees them");
    }

    #[test]
    fn a_concatenation_mark_waits_for_the_expansion_that_binds_it() {
        // `VGMC.MAC`'s ALPHA macro wraps an `.IRPC` around its own parameter
        // and concatenates the loop variable: `.BYTE A.'...Q` inside
        // `.IRPC ...Q,<STRING>` inside `.MACRO ALPHA STRING`.
        //
        // When ALPHA expands, `...Q` is not bound yet — `.IRPC` has not run.
        // Consuming the quote there because `A.` sits against it fused the
        // pair into the literal `A....Q` and left the inner `.IRPC` nothing to
        // substitute into, which is where the 14 undefined `CHAR....X` came
        // from. The mark belongs to whichever expansion actually binds a
        // neighbour.
        let src = "	.RADIX 16\nA.B = 05\nA.C = 06\n	.=0100\n\
                   	.MACRO ALPHA STRING\n	.IRPC ...Q,<STRING>\n\
                   	.BYTE A.'...Q\n	.ENDR\n	.ENDM\n	ALPHA ^/BC/\n";
        let pa = provider(&[("A.MAC", src)]);
        let mut a = Assembler::new(&pa);
        let img = a.assemble("A.MAC").expect("assembly failed");
        assert_eq!(bytes(&img, 0x0100, 2), vec![0x05, 0x06], "nested in a macro");

        // The same body at top level already worked, and is kept as the
        // control: a rule that fixed the nested case by never marking would
        // break this one.
        let bare = "	.RADIX 16\nA.B = 05\nA.C = 06\n	.=0100\n\
                    	.IRPC ...Q,<BC>\n	.BYTE A.'...Q\n	.ENDR\n";
        let pb = provider(&[("B.MAC", bare)]);
        let mut b = Assembler::new(&pb);
        let img = b.assemble("B.MAC").expect("assembly failed");
        assert_eq!(bytes(&img, 0x0100, 2), vec![0x05, 0x06], "bare .IRPC");
    }

    #[test]
    fn a_caret_delimited_argument_keeps_its_blanks() {
        // `^/text/` is MACRO-11's delimited argument: everything between the
        // delimiters is literal, spaces included. Lexing the interior as
        // ordinary tokens threw the padding away — whitespace survives a token
        // stream only as a `space_before` flag — so `^/  R  /` reached `.NCHR`
        // as ` R` and counted two characters instead of five.
        //
        // `AS2MSG.MAC` pads every message this way, `ASCIN ^/  RECORDS  /`.
        let p = provider(&[(
            "A.MAC",
            "	.=0100\n	.MACRO CNT STRING\n	.NCHR ..C,<STRING>\n	.BYTE ..C\n\
             	.IRPC ..5,<STRING>\n	.BYTE 055\n	.ENDR\n	.ENDM\n\
             	CNT ^/AB C/\n	CNT ^/  R  /\n",
        )]);
        let mut a = Assembler::new(&p);
        let img = a.assemble("A.MAC").expect("assembly failed");

        // Four interior characters, four iterations — this already worked, and
        // is here so a fix that over-corrects by padding is caught too.
        assert_eq!(bytes(&img, 0x0100, 5), vec![0x04, 0x55, 0x55, 0x55, 0x55]);
        // Two leading and two trailing blanks around one letter: five.
        assert_eq!(
            bytes(&img, 0x0105, 6),
            vec![0x05, 0x55, 0x55, 0x55, 0x55, 0x55]
        );
    }

    #[test]
    fn vctrs_writes_the_absolute_counter_not_the_section_one() {
        // `.VCTRS ADDR,...` names an absolute address, so it must set the
        // absolute counter. Setting the open section's counter instead made
        // the section look as though it ran from its base all the way up to
        // the vectors — `AS2TST.MAC` ends with `.VCTRS 8FFA,...` inside its
        // `.CSECT`, and its size came out as the distance to `0x9000` rather
        // than the 0x60D bytes it actually holds.
        let p = provider(&[(
            "A.MAC",
            "	.CSECT S\n	.BYTE 011,022,033\nSYM1::	.BYTE 044\n\
             SYM2 = 01234\n	.VCTRS 0FFA0,SYM1,SYM2\n",
        )]);
        let mut a = Assembler::new(&p);
        let img = a.assemble("A.MAC").expect("assembly failed");

        // Four bytes went into the section, and only those four.
        assert_eq!(a.sec_size["S"], 4, "the vectors must not stretch the section");
        let base = a.sec_base["S"];
        assert_eq!(bytes(&img, base, 4), vec![0x11, 0x22, 0x33, 0x44]);

        // The words landed at the absolute address, little-endian.
        let sym1 = base + 3;
        assert_eq!(
            bytes(&img, 0xFFA0, 4),
            vec![(sym1 & 0xFF) as u8, (sym1 >> 8) as u8, 0x34, 0x12]
        );
    }

    #[test]
    fn two_units_contributing_to_one_section_concatenate() {
        // A `.CSECT` has no address of its own. Two modules that open the same
        // one are appending to a single region, so the second starts where the
        // first stopped — the property that makes fifteen separately-assembled
        // modules link into one image.
        let p = provider(&[
            ("A.MAC", "	.CSECT SHARED\n	.BYTE 011,022,033\n"),
            ("B.MAC", "	.CSECT SHARED\n	.BYTE 044,055\n"),
        ]);
        let mut a = Assembler::new(&p);
        let img = a.assemble_units(&["A.MAC", "B.MAC"]).expect("assembly failed");
        let base = a.sec_base["SHARED"];
        assert_eq!(base, SECTION_ORIGIN, "the first section starts at the origin");
        assert_eq!(bytes(&img, base, 5), vec![0x11, 0x22, 0x33, 0x44, 0x55]);
        assert_eq!(a.sec_size["SHARED"], 5, "the section is as big as both parts");
    }

    #[test]
    fn sections_are_laid_out_end_to_end_in_first_encounter_order() {
        // Layout is concatenation in the order the sections are met, which is
        // link order because the roots are read in link order. A symbol defined
        // inside one resolves to base+offset everywhere, including from a unit
        // that cannot see the definition.
        let p = provider(&[
            ("A.MAC", "	.CSECT ONE\n	.BYTE 0,0,0,0\nMARK::	.BYTE 099\n"),
            ("B.MAC", "	.CSECT TWO\n	.BYTE 077\n"),
            ("C.MAC", "	.=0A000\n	.WORD MARK\n"),
        ]);
        let mut a = Assembler::new(&p);
        let img = a
            .assemble_units(&["A.MAC", "B.MAC", "C.MAC"])
            .expect("assembly failed");

        assert_eq!(a.sec_base["ONE"], SECTION_ORIGIN);
        assert_eq!(a.sec_base["TWO"], SECTION_ORIGIN + 5, "TWO follows ONE's five bytes");
        assert_eq!(bytes(&img, SECTION_ORIGIN + 5, 1), vec![0x77]);

        // The symbol is base+offset, and the third unit sees it.
        let mark = SECTION_ORIGIN + 4;
        assert_eq!(a.globals.get("MARK"), Some(&mark));
        assert_eq!(bytes(&img, 0xA000, 2), vec![(mark & 0xFF) as u8, (mark >> 8) as u8]);
    }

    #[test]
    fn asect_returns_to_the_absolute_counter_and_a_blank_csect_is_its_own_section() {
        // Two rules at once, because `AS2TST.MAC` exercises both: it opens a
        // bare `.CSECT` *and* a named one, so "no operand" cannot be read as
        // "absolute"; and `.ASECT` goes back to the absolute counter where it
        // left off rather than to zero.
        let p = provider(&[(
            "A.MAC",
            "	.ASECT\n	.=0A000\n	.BYTE 011\n\
             	.CSECT\n	.BYTE 0AA,0BB\n\
             	.CSECT NAMED\n	.BYTE 0CC\n\
             	.ASECT\n	.BYTE 022\n",
        )]);
        let mut a = Assembler::new(&p);
        let img = a.assemble("A.MAC").expect("assembly failed");

        // `.ASECT` resumed at A001, not at 0 and not inside a section.
        assert_eq!(bytes(&img, 0xA000, 2), vec![0x11, 0x22]);
        // The blank section is a section, and a different one from NAMED.
        let blank = a.sec_base[&blank_section("A.MAC")];
        let named = a.sec_base["NAMED"];
        assert_eq!(blank, SECTION_ORIGIN);
        assert_eq!(named, SECTION_ORIGIN + 2, "NAMED follows the blank section's two bytes");
        assert_eq!(bytes(&img, blank, 2), vec![0xAA, 0xBB]);
        assert_eq!(bytes(&img, named, 1), vec![0xCC]);
    }

    #[test]
    fn globb_declared_externals_size_zero_page() {
        // `.GLOBB` is `.GLOBL` plus "and it is a byte". The value lives in
        // another unit, so this one cannot see it — the *declaration* is what
        // says the operand is short. Space Duel writes 42 of these across ten
        // modules for the shared scratch page.
        //
        // Note B.MAC has no `.ENABL AMA`. That is the point: `VGUTR2.MAC` has
        // no `.ENABL` at all and still declares two `.GLOBB` symbols, so a rule
        // that merely widened what AMA already allows would not reach it.
        // The control is a *second* symbol, `WORDY`, identical in every way
        // except that nothing ever declares it `.GLOBB`. It cannot be the same
        // symbol: byte-ness travels with the symbol across the whole link (see
        // the per-unit reset), so `ZP` is byte-sized everywhere once any unit
        // says so, and asking one unit to treat it as wide would be asking the
        // linker to hold two answers at once.
        let p = provider(&[
            ("A.MAC", "	.=0090\nZP::	.BLKB 1\nWORDY::	.BLKB 1\n"),
            ("B.MAC", "	.GLOBB ZP\n	.GLOBL WORDY\n	.=0A000\n	LDA ZP\n	LDA WORDY\n"),
            ("C.MAC", "	.=0B000\n	LDA ZP\n"),
        ]);
        let mut a = Assembler::new(&p);
        let img = a
            .assemble_units(&["A.MAC", "B.MAC", "C.MAC"])
            .expect("assembly failed");

        // Declared byte-sized: zero page, two bytes.
        assert_eq!(bytes(&img, 0xA000, 2), vec![0xA5, 0x90], "`.GLOBB` import is zero page");
        // The control. Same unit, same shape, declared `.GLOBL`: absolute. If
        // this ever shrinks, the directive is a synonym and proves nothing.
        assert_eq!(
            bytes(&img, 0xA002, 3),
            vec![0xAD, 0x91, 0x00],
            "a `.GLOBL`-only import stays absolute"
        );
        // And byte-ness reaches a unit that never declared it at all — which is
        // the case `COIN65.MAC` needs, since it uses `$CNCT` unprefixed and
        // only `ASTRD2.MAC` says the symbol is a byte.
        assert_eq!(
            bytes(&img, 0xB000, 2),
            vec![0xA5, 0x90],
            "`.GLOBB` reaches a unit that never declared it"
        );
    }

    #[test]
    fn ama_confines_the_byte_hint_to_the_units_own_globb() {
        // How far a `.GLOBB` reaches depends on `.ENABL AMA` in the unit doing
        // the referencing. Its own declaration always applies; one made in
        // another unit applies only while AMA is off.
        //
        // Measured from Space Duel. `A2EARO.MAC` enables AMA, declares
        // `.GLOBL ... GAME`, and never declares `GAME` byte-sized — the only
        // `.GLOBB GAME` in the build is `A2NAME.MAC`'s. The original assembles
        // `A2EARO`'s three plain uses absolute. `AS2COI.MAC` is the same shape
        // without AMA, and the original assembles its plain `$CNCT` uses zero
        // page.
        //
        // `WIDE` is the symbol under test and `OWNED` is the control that keeps
        // the rule honest: if AMA simply discarded the hint, `OWNED` would go
        // absolute too, and `A2EARO`'s own `.GLOBB TEMP7,ATRACT,LANG,...` are
        // zero page in the original.
        let p = provider(&[
            ("A.MAC", "	.=0090\nWIDE::	.BLKB 1\nOWNED::	.BLKB 1\n"),
            // An earlier unit says both symbols are bytes, and uses neither.
            ("B.MAC", "	.GLOBB WIDE,OWNED\n"),
            // AMA on. `WIDE` is declared here only `.GLOBL`; `OWNED` is
            // declared `.GLOBB` here as well as in B.
            (
                "C.MAC",
                "	.ENABL AMA\n	.GLOBL WIDE\n	.GLOBB OWNED\n	.=0C000\n	LDA WIDE\n	LDA OWNED\n",
            ),
            // Same declarations as C's `WIDE` case, but no AMA.
            ("D.MAC", "	.GLOBL WIDE\n	.=0D000\n	LDA WIDE\n"),
        ]);
        let mut a = Assembler::new(&p);
        let img = a
            .assemble_units(&["A.MAC", "B.MAC", "C.MAC", "D.MAC"])
            .expect("assembly failed");

        // The A2EARO case: AMA withdraws another unit's hint.
        assert_eq!(
            bytes(&img, 0xC000, 3),
            vec![0xAD, 0x90, 0x00],
            "under AMA, a `.GLOBB` made in another unit does not reach this one"
        );
        // The control. Same unit, same AMA, but declared `.GLOBB` here.
        assert_eq!(
            bytes(&img, 0xC003, 2),
            vec![0xA5, 0x91],
            "the unit's own `.GLOBB` still applies under AMA"
        );
        // The AS2COI case: without AMA the hint carries across units, which is
        // what the previous test establishes and this one must not undo.
        assert_eq!(
            bytes(&img, 0xD000, 2),
            vec![0xA5, 0x90],
            "without AMA, another unit's `.GLOBB` reaches this one"
        );
    }

    #[test]
    fn a_globl_declaration_exports_a_single_colon_definition() {
        // `.GLOBL X` plus `X:` is the long form of `X::`. Crystal Castles only
        // ever writes the shorthand, so honouring the shorthand alone passed
        // its gate for years; Space Duel separates the two everywhere, and
        // under the old rule `ASTRD2.MAC` published almost nothing to the
        // fourteen modules that import from it.
        //
        // The control is `PRIV`, defined identically but never declared: it
        // must stay private, or the test is only showing that everything is
        // global now.
        let p = provider(&[
            ("A.MAC", "	.GLOBL DECL\n	.=0100\nDECL:	.BLKB 1\nPRIV:	.BLKB 1\n"),
            ("B.MAC", "	.=0200\n	.BYTE DECL\n"),
        ]);
        let mut a = Assembler::new(&p);
        let img = a.assemble_units(&["A.MAC", "B.MAC"]).expect("assembly failed");

        assert_eq!(a.globals.get("DECL"), Some(&0x0100), "declared and defined -> exported");
        assert!(!a.globals.contains_key("PRIV"), "an undeclared `NAME:` stays private");
        assert_eq!(img.get(&0x0200), Some(&0x00), "the other unit resolves it");
    }

    /// Module scoping against the real corpus.
    ///
    /// **Correction to an earlier claim.** Comparing the units by grep showed
    /// three shared names — `$INTCT`, `ATRACT` and `START` — and I described
    /// them as deliberately separate variables that a naive merge would
    /// corrupt. That overstated it. All three of CRP's definitions sit inside
    /// `.IF NE,...TST`, and `CRP.MAC:45` sets `...TST = 0` ("SELF-TEST
    /// MODE... 0 = OPERATIONAL MODE"). In an operational build CRP defines
    /// none of them and uses `CG.MAC`'s globals, so there are **zero**
    /// private-symbol collisions and a shared table would in fact have worked.
    ///
    /// Scoping is still the right design — it is the rule the source declares,
    /// it costs nothing, and it is required for a self-test build — but the
    /// evidence for its necessity was weaker than I said.
    ///
    /// What this test asserts is what is actually true: units keep separate
    /// private tables, private names never leak to the shared one, and the
    /// `::` globals really are shared, which is what removed the link step.
    #[test]
    #[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
    fn units_keep_their_own_private_symbols() {
        let Ok(dir) = std::env::var("CHILL65_CORPUS") else {
            return;
        };
        let root = std::path::PathBuf::from(&dir);
        struct Two {
            dirs: Vec<std::path::PathBuf>,
        }
        impl SourceProvider for Two {
            fn load(&self, name: &str) -> Option<Vec<u8>> {
                for d in &self.dirs {
                    for c in [name.to_string(), format!("{name}.MAC")] {
                        if let Ok(b) = std::fs::read(d.join(&c)) {
                            return Some(b);
                        }
                    }
                }
                None
            }
        }
        let p = Two { dirs: vec![root.join("version-3"), root.clone()] };
        let mut a = Assembler::new(&p);
        a.assemble_units(&["CRF.MAC", "CRP.MAC", "CLS.MAC"])
            .expect("combined build failed");

        for u in ["CRF.MAC", "CRP.MAC", "CLS.MAC"] {
            eprintln!("{u}: {} private symbols", a.unit_locals[u].len());
            assert!(!a.unit_locals[u].is_empty(), "{u} defined nothing private");
        }

        // CRF's START is its own and is not shared. CRP's is self-test only,
        // so in an operational build it is simply absent.
        assert_eq!(a.unit_locals["CRF.MAC"]["START"], 0xE000, "CIN.MAC:5");
        assert!(!a.globals.contains_key("START"), "START must stay private");
        assert!(
            !a.unit_locals["CRP.MAC"].contains_key("START"),
            "CRP's START is inside .IF NE,...TST and ...TST = 0"
        );

        // The shared symbols really are shared: RS.KEY:: is defined in CRP and
        // referenced from CRF, which is what removed the need for a link step.
        assert!(a.globals.contains_key("RS.KEY"), "RS.KEY:: should be shared");
        assert!(!a.unit_locals["CRF.MAC"].contains_key("RS.KEY"));
    }

    #[test]
    fn enabl_ama_is_recorded_for_the_encoder() {
        let p = provider(&[("MAIN.MAC", "	.ENABL	AMA,LC\n")]);
        let mut a = Assembler::new(&p);
        a.assemble("MAIN.MAC").unwrap();
        assert!(a.ama, "AMA must be visible to task 11's encoder");
    }
}
