# Phase 0.1 — Self-modifying code gate

**Verdict: GO.** No self-modifying code. No computed control transfers. Closes open question **O2**.

Subject: `github.com/historicalsource/crystal-castles` @ `9e9673a`, all three source trees
(root, `version-2/`, `version-3/`). Analysis is of source text, pre-assembly.

---

## 1. Memory map

Established from the source's own directives and assertions, not assumed.

| Range | Contents | Evidence |
|---|---|---|
| `0004`–`00EF` | zero-page game variables | `CG.MAC` `.= 04`; guard `.IF GE,.-0F0 / .ERROR ; RAM overlap with sounds` |
| `00F0`–`00FF` | POKEY zero page | `CRP.MAC` `.=0F0` |
| `0202`–`0AFF` | lower RAM | `CG.MAC` `.=202`; guard `.IF GE,.-0C00 / .ERROR ; out of lower RAM` |
| `0B00`– | POKEY voice variables | `CRP.MAC` `.=0B00` |
| `8000`– | additional RAM (player areas, city storage, elevators) | `CG.MAC` `RAM.ST=8000` |
| `A000`–`FFFF` | **program image** | `CG.MAC` `WV.STR=0A000 ; start of split storage`; `CRF.MAC` `.= WV.STR` / `CTROM = .` |

Fixed points inside the program image: `VC.SN=0CCE0`, `VC.IN=0E000` (`CSTART.MAC`),
tune table at `0D540` (`CLS.MAC`).

The program image is ROM on the board. Byte-level self-modification is therefore
physically impossible *unless* code is copied into RAM and executed there — which
reduces the gate to two questions, both answered below.

## 2. Method

Five patterns, not one:

1. Indirect `JMP` (`JMP (addr)`, `JMP @`, `JMP I,`)
2. `PHA`/`PHA`/`RTS` dispatch (return-address synthesis)
3. Stores landing inside the program image
4. Code copied into RAM and executed there
5. Indexed jump tables with computed targets

Patterns 1–3 scanned directly. Pattern 4 is proved by exhaustion of pattern 5: if
every control transfer target in the source is a statically named label in the
program image, execution cannot reach RAM.

Two scan generations. The first (`smc.py`) matched only bare store opcodes and
classified RAM by filename heuristic — it missed every macro-mediated store (the
majority idiom in this codebase: `TRAI` alone appears 313 times against 980 `LDA`)
and ran on the root tree only. The second (`tools/writescan.py`) parses the macro
definitions in every file, determines which formal parameters each macro stores to
or jumps to (transitively, for macros like `TR24AI` that call `TRAI`), checks the
corresponding actual arguments at every call site, and classifies symbols by
region-tracking the location counter along the real assembly stream — following
`.INCLUDE` from the three build roots — rather than guessing per file. All figures
below are from the second scan.

Macro-argument *transfers* are covered the same way: `CMAC.MAC`'s `BINT` macro takes
a branch target as an argument (15 call sites, all passing local code labels), and
`JMPIN` takes a jump-table symbol. Both were found by the macro pass, not by the
opcode pass.

## 3. Results

| Check | Root | `version-2` | `version-3` |
|---|---|---|---|
| Indirect `JMP` opcodes | 0 | 0 | 0 |
| Bare stores checked | 1,293 | 1,294 | 1,328 |
| Macro-mediated stores checked | 657 | 655 | 650 |
| Stores hitting program-image symbols | **0** | **0** | **0** |
| Control transfers to named labels | 684 | 683 | 696 |
| …to local (`n$`) labels | 353 | 355 | 337 |
| Macro-arg transfers to RAM labels | **0** | **0** | **0** |
| Transfer targets that are computed expressions | **0** | **0** | **0** |
| Unresolved symbols after triage | 0 | 0 | 0 |

Every intermediate hit during scan development was triaged to a scanner defect, not
a source finding; the instructive ones:

- `P1.LIV`, `P1.SCO`, `FC.BV` — bare alias labels directly above `.BLKB` blocks; RAM.
- `TRAM $LAM $COINA` — reads the slam/coin hardware switches and stores to
  `$COINS: .BLKB 1 ;COIN SWITCH SHADOW`; a shadow-register copy, not SMC. The
  scanner had swallowed `CCN.MAC`'s documentation header — prose like
  `$COINA:  "MECHS" locations…` wrapped in `.REPT 0 … .ENDR` (a never-assembled
  doc-block idiom) — as label definitions.
- `EECOIN = EEAUX` — `EEAUX: .BLKB 1` lives in `CEEDEF.MAC`, included from
  `CG.MAC`'s zero-page section (the file says so in its own header); zero-page RAM.
  Caught only once region tracking followed the include stream.

## 4. The five indirect transfers

The entire game contains five sites where control goes through a table. All five use
a **named** table of `.WORD target-1` entries pointing at labels in the program image.

| Site | Table | Bound |
|---|---|---|
| `CMN.MAC:43` `JMPIN GM.JTB` | `GM.JTB` — 16 game states | clamped at site: `CMP #GM.STZ+1 / IFCS / TRAI 0 GM.STA` |
| `CSS.MAC:57` `JMPIN AR.JTB` | `AR.JTB` — 7 attract states | **no clamp at site** — invariant: `AR.INC` wraps at `AR.MAX`; see below |
| `CEN.MAC:1213` `JMPIN 1$` | local, 17 enemy states | clamped at site: `CMP #10 / IFCS / LDA #10` |
| `CRP.MAC:791` `STUNE` | `VKT`, with `STRET-1` pushed as return address | caller contract (voice/tune numbers); not clamped at site |
| `CRP.MAC:1948` `PKFUN1` | `PKDT` — POKEY function dispatch | masked at site: `AND #0F` |

Three of the five are bounded at the dispatch site; two (`AR.JTB`, `STUNE`) rely on
program invariants maintained by their writers. `AR.STA` in particular is
initialised to `0FF` ("one before init arrow state") and only brought into range by
`AR.INC`'s wrap at `AR.MAX`; the dispatch is guarded by a menu-mode check
(`EN.JBP` = `MA.BU2`). This is a game-logic invariant, not a local proof. The
emitted Rust `match` for these two sites should carry a default arm that traps (or
logs) rather than silently diverging, and the differential harness (Phase 3) will
confirm the invariant empirically.

`JMPIN` is a macro in `M6502.MAC` taking the table symbol as its argument:

```
;  jump indirect,  A has starting address of jump table, Y has index
.MACRO JMPIN A
	LDA Y,A+1
	PHA
	LDA Y,A
	PHA
	RTS
	.ENDM
```

Every index is range-limited in the source before dispatch.

## 5. Consequences for the plan

**§3 (IR).** "Jump tables as first-class labelled data rather than computed targets"
needs no recovery work — the source already spells them that way. The IR requirement
is satisfied by preserving what is written, not by inferring it.

**§4 (WASM buckets).** Bucket 3 ("dispatch table lowering to `call_indirect`") is
five sites, all statically bounded. These lower to a Rust `match` over a state
enum, not `call_indirect`. Bucket 3 is effectively empty.

**§5 (creep line).** The stated benefit — "unresolvable indirect jumps degrade
gracefully instead of blocking the build" — has no unresolvable indirect jumps to
handle. The interpreter remains justified as a migration mechanism and debugging
instrument, but not as insurance against this specific risk.

**O2.** Closed: no.

## 6. What this does not prove

- ~~Analysis is of source text before macro expansion.~~ **Resolved — the
  authoritative check has now been run.** With a working macro expander (Phase
  1), every store instruction in the fully expanded stream was recorded with its
  resolved target address across the whole program build (`CRF` + `CRP` + `CLS`):

  > **2,054 stores with a resolved target. Zero into `A000`–`FFFF`.**

  The Phase 0.1 verdict holds on expanded code, not merely on source text. Note
  this covers the instruction stream as actually assembled, including everything
  synthesised by `M6502.MAC` and `HLL65F.MAC` — the very material the original
  caveat was about. Reproduce with
  `CHILL65_ANALYSE=1 cargo run -p chill65-asm -- CRF.MAC CRP.MAC CLS.MAC …`.

  One limit remains, and it is narrower: stores whose operand cannot be resolved
  to a constant (indexed and indirect forms, where the target is computed at run
  time) are not covered by an address check and never can be by static analysis.
  Those are bounded separately by §2's pattern 5 result — all indirect transfer
  targets are statically named — and by the memory map, since the ROM window is
  not writable on the hardware regardless.
- `CRP.MAC` contains `.=.-1` — now understood exactly. Its `LDAL`/`LDAH` macros emit
  `.BYTE 0A9` (LDA-immediate opcode), then `.WORD addr`, then back the location
  counter up one byte so the next instruction overwrites the word's high byte:
  hand-rolled "load low byte of a symbol's address as an immediate". `LDAH` wraps
  the `.WORD` in `.ENABL M68`/`.DSABL M68` (6800 byte order — high byte first) to
  get the high byte instead. Assembly-time byte punning, not run-time modification —
  but the front end must reproduce overlapping emission exactly, and the IR needs
  an operand form for "low/high byte of symbol address" (used to synthesise the
  `STRET-1` return address in `STUNE`).
- `CRP.MAC` is heavily conditional (`.IF NE,...PRK`, `.IF EQ,...AME`, `.IF NE,...FRE`
  and others). The sound driver is assembly-time configurable; the front end must
  evaluate these, and *which* configuration matches the shipped ROM is a Phase 0.2
  question.
- Stack depth and interrupt-time stack use were not analysed. The `STUNE` dispatch
  synthesises a return address and runs `SEI` before `RTS`; the recompiler's calling
  convention must accommodate this rather than assuming JSR/RTS pairing.

## 7. Incidental findings for Phase 0.2 / 0.3

- **Three trees, not two.** The plan's §0.2 assumes a `version-2`/`version-3` choice.
  The repository root is a third, distinct tree.
- **`CRP.MAC` includes `CJTB.MAC`, which exists only in `version-3`.** The root and
  `version-2` trees cannot assemble the sound package as they stand. This is likely
  the plan's "one source file was missing" (§2), and it bears directly on version
  selection.
- **Build roots:** `CRF.MAC` (game program, includes `CSTART.MAC` then 20 modules),
  `C99.MAC` (castle data, includes the 16 `C**.DAT` files at `.=n*WV.SIZ+WV.STR`),
  `CRP.MAC` (sound, separate).
- **Files are VAX block-padded with NUL bytes** to 512-byte multiples. Every reader
  in the toolchain must strip them; standard `grep` treats the files as binary and
  reports nothing without `-a`.
- **The dialect is MACRO-11-shaped**, not a 6502 assembler: `.ASECT`, `.RADIX`
  (with trailing-dot decimal literals), `.IIF`, `.REPT`, `.ENABL M68`, `.NLIST`,
  `.SBTTL` are documented DEC MACRO-11; `.DEFSTACK`/`.PUSH`/`.POP` and formatted
  `.PRINT` are Atari extensions. The 6502 arrives as a *macro package* —
  `M6502.MAC`, "version 2 of 6502 general purpose macros, May 26 1983, send mail to
  [FXL], Franz Lanzinger" — over a built-in 6502 opcode layer with prefix addressing
  syntax (`I,` `X,` `Y,` `NY,` `Z,` `AY,`). Its own header instructs users to copy
  it per-project, which predicts the per-game dialect drift recorded in §2 of the
  plan. Two idioms the front end must handle: documentation prose wrapped in
  `.REPT 0 … .ENDR` (never assembled), and NUL padding on every file.

## 8. Reproduction

`tools/writescan.py` is the authoritative scan: macro-aware store/transfer analysis
with include-stream region tracking, all three trees. `tools/gate.py` (control-
transfer census) and `tools/smc.py` (first-cut write scan, root tree only, six
false positives, no macro coverage) are kept as the record of how the result was
reached, not as tools to rerun. No dependencies beyond Python 3.
