# The Atari coin-op assembler dialect

What the front end implements, why, and what it deliberately does not. Every
claim here is checked against the corpus or against Atari's own toolchain
(`historicalsource/atari-coin-op-assembler`, `atari_tools/`), not taken from a
MACRO-11 manual — the two differ in places that matter.

Status: all four build roots assemble. The data image is **byte-identical** to
`C99.LDA`; the program image is the right size and address range but not yet
byte-identical (task #13).

---

## 1. File format

| Trap | Detail |
|---|---|
| NUL block padding | Every file is padded to a 512-byte multiple with NUL. Only *trailing* NULs are padding; an interior NUL would be corruption, so the reader trims from the end rather than filtering globally. |
| `grep` lies | Because of the NULs, standard `grep` treats these as binary and prints nothing without `-a`. This produced two false "not present" conclusions during development. |
| `.MAC` line endings | CRLF. |
| `.DAT` line endings | **None at all.** All 34 contain nothing but printable text and NUL padding — `C00.DAT` is 5,120 bytes of which 421 are NUL and the rest is one unbroken run. Record structure was VAX filesystem metadata and was lost when the files were flattened. Their only directive is `.BYTE` (4,484 occurrences corpus-wide, nothing else), so each `.BYTE` begins a record and the reader reconstructs boundaries on that basis. |

## 2. Lexical rules

- **Symbols** may contain letters, digits, `.`, `$`, `_`. `$` is common *inside*
  names (`$COINA`, `$$CRDT`, `..SRC$`) and also terminates a local label.
- **Symbols are case-insensitive.** `HLL65F.MAC`'s `FND` pushes `...S1` and then
  tests `.if eq,...s1&2` in lower case. A case-sensitive table reports it
  undefined and the whole structured-control-flow package collapses.
- **Symbols truncate to six characters.** MACRO-11 stores names RAD50-encoded,
  three per 16-bit word; `PST65.MAC` encodes every opcode with `.RAD50 /NAME/`.
  The corpus obeys it exactly — of 914 defined symbols, 658 are exactly six
  characters and none is longer. It is load-bearing: `CCUBE.MAC:6` defines
  `CLENGT`, `CCUBE.MAC:200` references `CLENGTH`, and it resolves only because
  the seventh character is discarded. Likewise `INCLUDE` (`CG.MAC:340`) stores
  as `INCLUD`.
- **Numbers** are radix-sensitive (`.RADIX`). A trailing dot forces decimal
  regardless (`15.`), and `^H` / `^D` / `^B` / `^O` override per literal.
  Radix is *not* lexer state: macro bodies are re-lexed at the expansion site's
  radix, so a number token carries its raw text and resolves on demand.
- **Local labels** are `N$` (`10$:`), scoped between ordinary labels.
- **`'`** is both the concatenation operator (`B'COND` → `BNE`) and a literal
  delimiter in `.PRINT`. The lexer leaves it uninterpreted; the macro engine
  decides, where the context is known.
- **`.ASCII /text/`** takes whatever character follows the directive as the
  delimiter. The text is captured in the lexer, because tokenising and
  reassembling would lose its spacing.

## 3. Expressions

**Evaluation is strictly left to right, with no operator precedence.** This is
not a detail; the corpus depends on it in both directions:

- `M6502.MAC`'s `TR16AI` extracts a high byte with `FROM&^H0FF00/^H100`. Left to
  right that is `(FROM & 0xFF00) / 0x100`. Under conventional precedence `/`
  binds first and yields the **low** byte, silently corrupting every 16-bit
  immediate transfer in the game.
- `HLL65F.MAC` computes listing indentation as `..NST$+1*3+..SRC$` in `$INDEN`
  and `..NST$*3+..SRC$` in `$UNDEN`. Those are the same formula one nesting
  level apart only under left-to-right.
- The same file writes `<9.*3+..SRC$>` in angle brackets — necessary *because*
  left-to-right would otherwise subtract first.

Angle brackets are the only grouping construct. Arithmetic is 16-bit wrapping.

### Operators

| Operator | Meaning | Corpus usage |
|---|---|---|
| `+ - * /` | arithmetic | throughout |
| `&` | AND | 34 |
| `!` | OR | 16, all in `CCN.MAC` conditionals |
| `?` | **exclusive-or** | 3, all in `CG.MAC`'s ROM checksum equates |
| `<>` | grouping | HLL65F |
| unary `-`, `+` | | |

`?` was the last unknown. It is confirmed against all three shipped ROM images:

```
root  CHK01 = 082?7F?01  ->  0x82^0x7F^0x01 = 0xFC = byte at A024
v2    CHK01 = 73?1       ->  0x73^0x01      = 0x72 = byte at A034
v3    CHK01 = 0E7        ->  0xE7           = 0xE7 = byte at A034
```

(v2 and v3 sit sixteen bytes further along because their `CGR.MAC` emits a
second `.ASCII` string, `" PIRATES BEWARE"`.)

**Not implemented:** the word forms `.AND.` / `.OR.` / `.NOT.`. They are not
operators in this dialect — `.OR.` occurs once, in `HLL65F.MAC:51` as
`.IF IDN,<.A>,<.OR.>`, an identity test of a macro argument; `.NOT.` once inside
an `.ERROR` message; `.AND.` never. They also cannot lex as operators, since `.`
is a symbol character and `0F0.OR.0F` tokenises as a number followed by the
single symbol `.OR.0F`. A form that worked only when spaced would silently
misparse the unspaced one, so it is rejected loudly instead.

## 4. Directives

Implemented: `.ASECT` `.RADIX` `.BYTE` `.WORD` `.ASCII` `.ASCIZ` `.BLKB`
`.BLKW` `.INCLUDE` `.IF` `.IFF` `.IFT` `.IFTF` `.IIF` `.ENDC` `.REPT` `.IRP`
`.ENDR` `.ERROR` `.GLOBL` `.ENABL` `.DSABL` `.NOCROSS` `.END` `.MACRO` `.ENDM`
`.MEXIT` `.DEFSTACK` `.PUSH` `.POP` `.GETPOINTER` `.VCTRS` `.=`.

Parsed and ignored: `.TITLE` `.SBTTL` `.PAGE` `.LIST` `.NLIST` `.PRINT` `.REM`
`.MCALL` `.IDENT` `.EVEN` `.ODD` `.WARN`.

Points where the dialect surprised us:

- **`.ENDM` and `.ENDR` are interchangeable.** MACRO-11 terminates `.MACRO`,
  `.REPT`, `.IRP` and `.IRPC` alike with `.ENDM`; `.ENDR` is the repeat-specific
  synonym. The corpus mixes them: `CMR.MAC:111` and `CWV.MAC:743` close a
  `.REPT` with `.ENDM`, while `HLL65F.MAC:224`'s `.IRP` closes with `.ENDR`.
  Treating them as distinct made the body scanner run to end-of-file and swallow
  the remainder of six files — 223 errors from one mismatch.
- **`.=` moves backwards.** `.=.-1` backs the location counter up so the next
  emission overwrites. `CRP.MAC`'s `LDAL`/`LDAH` depend on it. Emission is into
  an address-keyed image, so **last write wins** — a resolution confirmed by
  reproducing Atari's thirteen documented ROM checksums.
- **`.RADIX`'s operand is always decimal**, whatever radix is in force;
  otherwise `.RADIX 10` under hex would mean sixteen.
- **`.REPT 0` wraps documentation.** `CCN.MAC` puts pages of English prose
  inside it. The prose is not valid syntax and must never be parsed — only
  skipped, by scanning leading tokens alone.
- **`.ERROR` reports; it does not abort.** `CRF.MAC:72` carries an unconditional
  `.error ; dummy` in shipped source, so a fatal reading makes the real program
  unbuildable. The form is `.ERROR expr ; message` and both halves matter:
  HLL65F writes `.ERROR ...S0 ; BRANCH OUT OF RANGE`.
- **Unknown dot-directives warn rather than fail.** `CRP.MAC:56` carries
  `.GLOBB`, a typo for `.GLOBL` that shipped — and which is load-bearing
  *because* it failed, since had it worked CRP's `$INTCT` and `ATRACT` would
  have gone global.
- **`.GETPOINTER STACK,SYM`** sets `SYM` to a stack's remaining depth, so
  `HLL65F`'s `HLL65` macro can assert the PC stack came back balanced.
- **`.VCTRS ADDR,W1,W2,…`** positions the counter and lays down words;
  `CRF.MAC:77` uses it for the 6502 vectors at `FFF8`.
- **`.IRP`** occurs exactly once (`HLL65F.MAC:224`) and generates the counted
  shift macros `ASLS`/`LSRS`/`INXS`/… which the corpus uses roughly a hundred
  times. One occurrence, entirely load-bearing.

### Conditions

Census across all three trees: `NE` 369, `EQ` 165, `GE` 103, `NDF` 56, `GT` 16,
`NB` 12, `LT` 6, `IDN` 4, `B` 2. `LE`, `DF` and `DIF` never occur but are
implemented as the complements of conditions that do.

They fall into three families, and the family decides how the operand is *read*:

- **numeric** (`EQ NE GT GE LT LE`) — evaluate and compare against zero,
  **signed**. `CG.MAC`'s guards (`.IF GE,.-0F0`) depend on this: the subtraction
  wraps when the counter is below `0F0`, so an unsigned comparison fires the
  guard backwards on correct source.
- **symbol** (`DF NDF`) — is a symbol defined?
- **text** (`B NB IDN DIF`) — inspect a macro argument's raw text.

## 5. Macros

- Parameter lists use commas *or* spaces (`.MACRO TRAI FROM,TO` but
  `.MACRO ADAI VAL MEM`), and so do call sites (`TRAI 2*EN.MAX EN.NUM`). Since
  the lexer discards whitespace, argument splitting relies on retaining
  *whether* whitespace was present.
- **Parameter names collide with addressing-mode prefixes.** `CMAC.MAC` declares
  `.MACRO CMPIN A,B`, and `A,` lexes as a mode prefix. No *symbol* in the corpus
  is named after a mode (checked across all three trees), but parameters are.
- A `?` formal mints a fresh label **only when the actual is omitted**.
  `CMAC.MAC`'s `.MACRO BINT INTVAL,?B,?C` is called as `BINT 2,40$` — the caller
  names the branch target while `C` stays internal.
- A `?` formal used as a *definition* (`B:` at the end of `INC16`'s body) must
  substitute too, or the minted label is referenced and never defined.
- Missing arguments bind to empty, which `.IF NB,<COND>` relies on.
- **Macros can define macros.** `HLL65F`'s `DEFIF IFCC,BCS` and
  `DEFEND PLEND,BMI,BPL` generate the whole `IFEQ`/`PLEND` family.
- Macro names may begin with `.` (`CRP.MAC` defines `.MACRO .TUNE`). Directives
  take precedence, so a macro cannot shadow `.BYTE`.
- **Macro namespaces are per assembly unit, unconditionally** — there is no
  `::` equivalent for macros. `LDAL` and `LDAH` are each defined twice with
  different bodies: `HLL65F.MAC` by expression arithmetic (`LDA #<.1.>&255.`)
  and `CRP.MAC` by byte punning (`.BYTE 0A9 / .WORD ...1 / .=.-1`). They are not
  equivalent — CRP's writes a third byte and rewinds so the next emission
  overwrites it.

## 6. HLL65F structured control flow

The package the whole recompiler premise rests on. Mechanism, decoded from
`HLL65F.MAC` and verified by assembling against the real file:

```
LOC type   $INDEN, then push (current `.`, type) onto the PC stack.
IFXX .1.   `LOC 0` then `.1. .` — emits the branch with a PLACEHOLDER
           operand pointing at itself.
FND        pops (P1,S1); `. = ...P1+1` rewinds to the operand byte,
           `...S0 = ...P0-...P1-2` is the relative offset, range-checked
           against +127/-128, `.BYTE ...S0` writes OVER the placeholder,
           then `. = ...P0` restores.
BEGIN      `LOC 2`
..END      pops the loop start; emits `.1. P0` (backward branch), or
           `.2. .+5` then `JMP P0` when out of range.
```

`DEFIF IFEQ,BNE` means `IFEQ` emits **BNE** — the inverse condition, because the
branch jumps *over* the block.

**This is the source of the 624 overlapping writes in `CRF.LDA`.** `FND` rewinds
the location counter to overwrite a placeholder operand. Any comparison against
the oracle must therefore use the *resolved* image, never the record stream.

Verified output against the real package:

| Construct | Bytes |
|---|---|
| `IFEQ … ENDIF` | `a9 00 d0 01 ea 60` — `BNE +1` skips exactly the NOP |
| `BEGIN … PLEND` | `ea 30 fd` — `BMI -3` to the loop top |
| `IFEQ … ELSE … ENDIF` | `a9 00 d0 04 ea 4c 09 a0 aa 60` |
| 200-byte block | `.ERROR: BRANCH OUT OF RANGE (512 / 0x0200)` |

## 7. Instruction encoding

The addressing-mode set is Atari's own, from `OPC65.MAC`'s `AMCHR`:

```
NX  Z  I  A  NY  ZX  AY  AX  S  AC  N  ZY  X  Y
```

`PST65.MAC` calls the relative mode `R` where `OPC65.MAC` calls it `S`; they are
the same. `X` and `Y` are auto-sizing forms resolved by AMA.

Two operand syntaxes coexist. The prefix form (`LDA I,FROM`, `STA NY,PKPTR`) is
what `OPC65.MAC` defines; the abbreviations are what `M6502.MAC`'s own header
tells its users to write:

> Use the abbreviations #,@,(X), i.e.
>   a: `TRAM ARG1(X) @ARG2(Y)` works, but
>   b: `TRAM <X,ARG1> <NY,ARG2>` doesn't.

So `#expr` is immediate, `@expr` indirect, trailing `(X)`/`(Y)` indexed, and
`@expr(Y)` indirect-indexed. **The closing parenthesis is optional** —
`CEN.MAC` writes `STA EN.IY(X` and `LDA EN.HP(X` in all three revisions, a typo
the original assembler tolerated and whose ROMs work.

`.ENABL AMA` ("auto zero page management") makes an operand below `0x100` take
the zero-page form, changing the instruction from three bytes to two and moving
every address after it. **The mode is chosen once in pass one and replayed in
pass two**: a forward reference is unresolved in pass one so the mode is
absolute, and letting pass two decide afresh would shorten the instruction and
shift everything downstream.

The opcode table is explicit rather than a reimplementation of PST65's
`base + 4 × slot` arithmetic — 151 opcodes across 56 mnemonics, exactly the
documented NMOS 6502 count, with PST65's `AM` field used as the authority for
which modes each mnemonic accepts. A mnemonic absent from the table is a macro,
not an instruction.

## 8. Assembly units and scoping

The original build ran four assemblies and two link steps. We assemble all four
roots in one pass over each, giving each unit its own private symbol table,
macro table, radix, AMA/M68 flags, conditional stack and local-label scope,
while sharing the image and the `::`/`==` globals. That removes the link step:
`RS.KEY::` is defined in `CRP.MAC` and referenced from `CRF`'s side, and simply
resolves.

The scoping rule is the source's own: `NAME:` and `NAME=` are private to the
unit, `NAME::` and `NAME==` are shared, `.GLOBL` declares an external.

**An honest note on the evidence.** Comparing the units by grep showed three
shared names — `$INTCT`, `ATRACT`, `START` — which were reported as separate
variables a naive merge would corrupt. That overstated it: all three of CRP's
definitions are inside `.IF NE,...TST`, and `CRP.MAC:45` sets `...TST = 0`
("SELF-TEST MODE... 0 = OPERATIONAL MODE"). An operational build has **zero**
private-symbol collisions and a shared table would have worked. Scoping is still
right — it is what the source declares, it costs nothing, and a self-test build
needs it — but it was not load-bearing in the way first claimed. Macro scoping
*is* load-bearing: `LDAL`/`LDAH` are genuinely defined twice, unconditionally.

## 9. Not implemented

| Form | Why |
|---|---|
| `.AND.` / `.OR.` / `.NOT.` | Not operators in this dialect, and unlexable as such. §3. |
| `.IRPC` | Used by Atari's toolchain sources but never by a game. |
| `.RAD50` as a directive | Only the six-character *limit* matters here; no game emits RAD50 data. |
| Relocatable output | Everything is `.ASECT` with explicit `.=`. There is nothing to relocate. |
