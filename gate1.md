# Phase 1 gate — reassembly against the `.LDA` oracle

**Status: PASSED. Both images are byte-identical to the original toolchain's own
output.**

| Image | Oracle | Result |
|---|---|---|
| Castle data (`C99`) | `C99.LDA` | **16,384 of 16,384 — byte-identical** |
| Program (`CRF`+`CRP`+`CLS`) | `version-3/CRF.LDA` | **24,576 of 24,576 — byte-identical** |

Reproduce with:

```
CHILL65_CORPUS=/path/to/crystal-castles \
  cargo test -p chill65-asm --test gate -- --ignored --nocapture
```

That command is the Phase 1 gate, and `tests/gate.rs` asserts equality — not a
tolerance. The oracle is the assembled output shipped in the source tree, so no
ROM set is required and nothing game-derived enters this repository.

## 0. The two causes that closed the last 1,696 bytes

The program image sat at 93.10% with 1,696 differing bytes in 75 runs. They had
exactly **two** causes, which is why they collapsed together rather than one at
a time.

### 0.1 `.GLOBL` declares an import, and imports size absolute — 1,687 bytes

A symbol a unit declares `.GLOBL` *and does not define* is an import: the
assembler cannot know its value fits the zero page, so the operand is absolute
regardless of what the value later proves to be.

The corpus demonstrates both sides of the rule four lines apart:

```
CRP.MAC:52   .GLOBL  TUNTAB,DOTPL,EN.HEI,SN.NUM      ← a working declaration
CRP.MAC:56   .GLOBB  $INTCT,ATRACT                   ← a typo that never took
```

and the oracle assembles accordingly:

| Reference | Address | Oracle bytes | Mode |
|---|---|---|---|
| `STA SN.NUM` (declared) | 00BA | `8d ba 00` | absolute |
| `LDA ATRACT` (mis-declared) | 00B9 | `a5 b9` | zero page |
| `LDA $INTCT` (mis-declared) | 00A3 | `a5 a3` | zero page |

**The misspelling changed instruction sizes**, and reproducing the original
byte-for-byte means reproducing its consequences.

Both conditions matter: a unit that declares `.GLOBL X` *and* defines `X` is
exporting, not importing, and sizes normally. `CG.MAC` does exactly that for
`$INTCT`.

This corrects the earlier fix recorded in §3.2 below, which used "not defined in
this unit" — right for the `SN.NUM` case, backwards for the `$INTCT` case.

### 0.2 `.IFT` / `.IFF` / `.IFTF` are subconditionals, not an if/elseif chain — 9 bytes

They test the enclosing `.IF`'s result — "if true", "if false", "if true or
false" — and may repeat within a single frame. `CCN.MAC` does exactly that:

```
315  .IF EQ,MECHS-1
323  .IFF          ← MECHS > 1
344  .IFTF         ← any number of mechs
370  .IFT          ← MECHS == 1 again
372  .IFF          ← MECHS > 1 again
374  .IFTF
```

Modelling them as a chain with a "has any branch been taken" flag makes the
second `.IFT` activate because the earlier `.IFF` fired — so both arms of a
two-way choice assemble. The symptom was nine bytes differing by exactly `+0x10`,
the zero-page versus zero-page,X opcode gap: `STA Z,$CNSTT` where the oracle had
`STA ZX,$CNSTT`.

Both causes are pinned by regression tests, and the second asserts both
polarities so the fix cannot simply be inverted.

---

## 1. What passing the data image establishes

`C99.LDA` matching exactly is not a small result. Producing those 16,384 bytes
exercises the whole front end: NUL de-padding, the `.DAT` files' absent record
structure, `.RADIX` switching between 10 and 16 around every include, the
location counter across `.=` jumps and 16 interleaved playfield blocks, `.REPT`
padding, expression evaluation, and the two-pass driver. Any error in those
would show up here, and none does.

## 2. Where the program image differed (historical)

All 1,696 differing bytes are **at or after `CB8F`**, in 75 runs. Everything
below that — `A000`–`CB8E`, about 10.9 KB, the whole of `CRF`'s own code — is
byte-identical.

| 4K page | Differing bytes | What lives there |
|---|---|---|
| `C000` | 590 | end of `CRF`, then `CRP` from `CCE0` |
| `D000` | 1,096 | `CRP`, and `CLS`'s tune table from `D540` |
| `E000` | 1 | fixed ROM |
| `F000` | 9 | fixed ROM, near the vectors |

So the residue is confined to the **separately assembled sound package**.

Structurally the runs fall into two kinds:

- **38 single-byte runs** — operands, not content. Their deltas are small and
  varied (`+16` ×10, `−2` ×9, `+1` ×4, `−8` ×2, `+32` ×2 …), which is the
  signature of forward references into regions whose size we have not yet got
  exactly right.
- **16 runs of 16 bytes or more**, covering 1,590 of the 1,696 bytes — genuine
  content divergence, starting at `CDB2` and dominated by `CE70`–`D0EE` (639
  bytes).

## 3. Diagnosed and fixed during this gate

Two causes were found and corrected, together worth 4,677 bytes (74.07% → 93.10%).

### 3.1 Implicit `.WORD` — 4,582 bytes

`CEN.MAC:1803` writes the enemy-state jump table with **no directive at all**:

```
;  table of addresses for state handlers
1$:	10$-1,11$-1,12$-1,13$-1,14$-1,15$-1
	16$-1,17$-1,18$-1,19$-1
	20$-1,21$-1,22$-1,23$-1,24$-1,25$-1,26$-1
```

In MACRO-11 a statement whose operator field holds an expression rather than an
opcode is an implicit `.WORD` list. Seventeen entries, 34 bytes. We emitted
nothing, so every address after it was 34 low — and the 128 single-byte
differences seen at that stage were all exactly `+0x22`.

It passed silently because the line begins with a local-label token rather than
a symbol, so the dispatcher fell through without even counting it as unhandled.

### 3.2 Cross-unit references must size as externals — 95 bytes

```
ours:   85 ba      = STA $BA    (zero page, 2 bytes)
oracle: 8d ba 00   = STA $00BA  (absolute,  3 bytes)
```

This is a direct consequence of assembling everything in one run instead of
separately-plus-link. The original assembled `CRP` on its own, where a symbol in
the *game's* zero page is an undefined external — and an external can only be
sized absolute. In a combined build `CRF` has already defined it, so `AMA`
shrinks the instruction to two bytes and shifts everything after.

Fixed by tracking, per unit, which symbols that unit itself defines: an operand
touching anything else is sized absolute regardless of its value. The encoded
*value* still comes from the shared table; only the sizing decision changes.

**This is the one real cost of dropping the link step**, and it is worth stating
plainly: removing the linker did not remove the semantics of separate assembly,
it moved them into the sizing rule.

## 4. What the residue turned out to be (historical)

The lead recorded here was that the residue lay in `CRP.MAC`'s assembly-time
configuration flags. That was **wrong** — the flags all evaluated correctly, as a
conditional trace confirmed. The causes were the two in §0. Kept as written
because the reasoning that led there is worth seeing:

The residue is concentrated in `CRP.MAC`, which is by far the most
conditional-heavy file in the corpus. It is built around assembly-time
configuration flags — `...TST`, `...PRK`, `...AME`, `...FRE`, `...SV3`,
`...SV4`, `...PRI`, `...DRE` and more — with blocks like

```
.IF	NE,...SV4		;IF SPLIT VOICE 4...
.IF	GE,...PRI-2		;TEST FOR PRIORITY 2
	JSR	PKOUT6		;OUTPUT PRI. 2, VOICE 4B
.ENDC
```

A single flag evaluated differently from the original includes or excludes a
block and shifts everything after it, which matches the observed shape: a few
large content runs plus a scatter of operand fixups.

**The next step is therefore to compare our evaluation of `CRP.MAC`'s
configuration flags against what the original build used**, not to chase
individual bytes. Concretely: dump every `.IF` in `CRP.MAC` with the value we
computed for its condition, and look for one whose result changes a block size
by the observed amounts.

Two smaller leads:

- The first difference, at `CB8F`, is `JSR $CE7E` where the oracle has
  `JSR $CE7C` — a forward reference from `CRF` (`SC.INE+0x117`) into `CRP`,
  two bytes off. That is a symptom of the same sizing question, seen from the
  other side.
- `CLS.MAC`'s tune table from `D540` accounts for much of the `D000` page. It
  should be checked separately, since it is data rather than code and may have
  an independent cause.

## 5. Regression guard

`crates/chill65-asm/tests/gate.rs` now asserts **equality** on both images. The
failure report is kept richly instrumented anyway — first differing address,
both bytes, surrounding context, run structure, and the nearest preceding symbol
with its defining unit — because if a regression ever lands, "images differ"
would tell nobody anything.

## 6. The other two gate deliverables

Both are done and are recorded where they belong:

- **Self-modifying code, re-checked post-expansion.** `smc-gate.md` §6 carried a
  caveat that the Phase 0 scan could only read source text. With a working macro
  expander the check has now been run over the fully expanded stream: **2,054
  store instructions with a resolved target, zero of them into `A000`–`FFFF`**.
  The Phase 0.1 verdict holds. Recorded in `smc-gate.md`.
- **Open question O3.** Measured in `codegen-readiness.md`: **64.4% of branches
  come from HLL65F structured control flow**, 5.3% from other macros, 30.4%
  hand-written.
