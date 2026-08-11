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
    /// Current local-label region. `N$` labels are scoped between ordinary
    /// labels, so the same `10$` recurs all through the corpus.
    local_scope: u32,
}

type Line<'t> = &'t [Token];

impl<'a> Assembler<'a> {
    pub fn new(provider: &'a dyn SourceProvider) -> Self {
        Assembler {
            provider,
            locals: HashMap::new(),
            globals: HashMap::new(),
            global_decls: HashSet::new(),
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
    pub fn assemble_units(&mut self, roots: &[&str]) -> Result<BTreeMap<u16, u8>, Vec<String>> {
        for pass in 1..=2 {
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
                // Per-unit state. Globals and the image deliberately persist.
                self.loc = 0;
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
                self.defined_here.clear();
                self.locals = self
                    .unit_locals
                    .get(*root_name)
                    .cloned()
                    .unwrap_or_default();
                self.global_decls.clear();

                self.ir_unit = root_name.to_string();

                let Some(raw) = self.provider.load(root_name) else {
                    self.errors
                        .push(format!("cannot open root source {root_name}"));
                    continue;
                };
                self.run_source(root_name, &raw);
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

        // Choose the mode once, in pass one, and replay it in pass two.
        let mode = if self.pass == 1 {
            match encode::resolve_mode(mnemonic, written, has_operand, size_value, self.ama) {
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
            Dir::Asect | Dir::Nocross | Dir::Ignored => {}

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

            Dir::Globl => {
                if self.pass == 1 {
                    for f in split_commas(args) {
                        if let Some(Tok::Symbol(s)) = f.first().map(|t| &t.tok) {
                            self.global_decls.insert(intern(s));
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
                        match inner.first() {
                            Some(first) => {
                                let text: String =
                                    inner.iter().map(crate::macros::token_text).collect();
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

    /// Define a symbol in the private or shared table.
    ///
    /// `NAME:` and `NAME=` are private to the unit; `NAME::` and `NAME==` are
    /// shared. The distinction is already carried on the token — it is the
    /// source's own scoping rule, not one we invent.
    fn define(&mut self, name: &str, value: u16, global: bool) {
        let key = intern(name);
        self.defined_here.insert(key.clone());
        if global {
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
