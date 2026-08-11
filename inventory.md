# Phase 0.3 — historicalsource archive inventory

**Answer to the scope question: about twenty titles, not five and not twenty-five —
and they split cleanly into two dialect generations.** Detail in §5–§6.

Surveyed: every repository under `github.com/historicalsource` — **329 repos**,
each one's full recursive file tree fetched via the GitHub API. Complete per-repo
record in `inventory-full.tsv` (330 lines, one header + one row per repo). This
document summarises; the TSV omits nothing.

`historicalsource` is a **user account, not an organisation** (`/orgs/` 404s,
`/users/` works) — worth knowing for any future tooling.

---

## 1. Method and cost

- 4 requests to page the repo list, 329 to fetch trees, 1 failure.
- Run authenticated via `gh` (5,000 req/hr). Unauthenticated (60/hr) this survey
  would have been impossible in one pass — the task anticipated sampling, but full
  coverage turned out to be affordable.
- Classification is by file-extension signature and filename markers computed
  offline from the fetched trees. **No repository was cloned.**

## 2. Shape of the archive

| Class | Repos | What it is |
|---|---|---|
| Modern C/C++ games | 104 | Quake 2, Descent 2, Abuse, Blood, AvP, Arx Fatalis … |
| Other | 97 | Sierra AGI/SCI adventures, mixed assets, tooling |
| Other assembly | 56 | Williams (`joust`, `defender`, `robotron`, `sinistar`, `stargate`), Midway TMS34010 (`nba-jam`, `smashtv`, `total-carnage`, `trog`), Atari 7800 homebrew (`*-7800`), BBC/Electron, MS-DOS, GW-BASIC |
| Infocom (ZIL/ZAP) | 48 | `zork`, `amfv`, `beyondzork`, `shogun`, `ballyhoo` … |
| **Atari coin-op (`.MAC`)** | **23** | the relevant set — 20 genuine, see §3 |
| API error | 1 | `mk2` — tree fetch failed, unclassified |

Three of the 23 `.MAC`-classed repos are **false positives** carrying an incidental
`.MAC` file: `descent2` (4), `narc` (5), `ucb-csrg-bsd` (13, and 311 MB of BSD).
Excluded from the table below, retained in the TSV.

## 3. The Atari coin-op set

Twenty repositories: nineteen titles plus the toolchain.

| Repo | KB | `.MAC` | HLL package | Vector-generator macros | `COIN65` |
|---|---|---|---|---|---|
| `crystal-castles` | 229 | 96 | **HLL65F** | — | — |
| `star-wars` | 419 | 67 | **HLL69F** | AVGROM, VGAN, VGTST, WSVGAN | — |
| `atari-coin-op-assembler` | 2520 | 67 | — | — | yes |
| `gravitar` | 205 | 53 | **HLL65 + HLL65F** | VGAN, VGMC, VGUT | yes |
| `guts` | 1318 | 51 | — | — | — |
| `space-duel` | 184 | 26 | **HLL65F** | ASCVG, VGAN, VGMC, VGUTR2 | yes |
| `centipede` | 89 | 24 | — | — | yes |
| `tempest` | 173 | 24 | **HLL65** | ALVGUT, ANVGAN, ASCVG, VGMC | yes |
| `asteroids-deluxe` | 110 | 23 | — | — | yes (`DCIN65`) |
| `black-widow` | 152 | 23 | **HLL65F** | VGMC16, VGUT16, BBAVGD, BBAVGT | yes |
| `tube-chase` | 73 | 16 | — | — | — |
| `red-baron` | 100 | 13 | — | VGAN, VGMC, VGUT | — |
| `liberator-2` | 115 | 13 | **HLL65** | — | yes |
| `warlords` | 60 | 12 | **HLL65** | — | — |
| `battlezone` | 104 | 11 | — | VGMC, VGUT | yes |
| `millipede` | 80 | 11 | **HLL65F** | — | yes |
| `liberator` | 91 | 8 | **HLL65** | — | yes |
| `maze-invaders` | 90 | 7 | — | — | yes |
| `tank-8` | 33 | 6 | — | — | — |
| `quizshow` | 21 | 4 | — | — | — |

**Years were not determined.** Each repo carries Atari release `.DOC` files of the
same kind that dated Crystal Castles precisely (`372X1.DOC` → 6-20-83), so this is
cheap follow-up, but nothing here is asserted from memory.

## 4. The toolchain is in the archive

`atari-coin-op-assembler` (2.5 MB) is not a game. It contains four trees —
`atari_tools/`, `coin-op/`, `centipede/`, `centipede-reassembled/` — and
`atari_tools/` holds **Atari's own development tools in source form**:

- `OPC65.MAC` / `OPC65.OBJ` — the **6502 opcode tables**
- `OPC68.MAC` / `OPC68.OBJ` — 6800 opcode tables (explaining `.ENABL M68`)
- `MAC69.MAC`, `A3MC.M65`, `M6TA.M69` — assembler components
- `LINK0.MAC`, `LNKOV1–4.MAC` (+ `.OBJ`), `LNKV2B.OBJ`, `LNKLNK.CTL` — **the LINKM
  linker**, in overlays
- `EDIT.MAC`, `PIP.MAC`, `BATCH.MAC`, `BA.SYS`, `ODT.OBJ` — editor, file copy,
  batch, debugger
- split into `e2_tools/` and `e3_tools/` generations

This bears directly on two open items:

- **O1** (task #5): the dialect is not something to be inferred from game sources —
  the authoritative opcode tables and directive implementation are here to be read.
- **Task #12** discovered that a LINKM-equivalent linker is needed. Its source is
  here too.

## 5. Dialect generations

The decisive split is the structured-control-flow macro package:

**With HLL65/HLL65F/HLL69F — 10 titles:** crystal-castles, star-wars, gravitar,
space-duel, tempest, black-widow, liberator, liberator-2, warlords, millipede.

**Without — 9 titles:** centipede, asteroids-deluxe, battlezone, red-baron,
tube-chase, maze-invaders, tank-8, quizshow, guts.

This matters more than anything else in the survey. The whole project premise
(plan §1: "the original programmers wrote structured control flow, and it survives
in the source") holds for **half** the archive. For the other half the relooper
path of plan §4 is not a fallback for odd routines — it is the primary mechanism.

Three shared library families are visible, which is the real argument for a
retargetable toolchain:

- **`HLL65` / `HLL65F` / `HLL69F`** — structured control flow, 10 titles.
- **`VGMC` / `VGUT` / `VGAN` / `ASCVG`** — vector generator, 7 titles
  (gravitar, space-duel, tempest, black-widow, red-baron, battlezone, star-wars).
  Confirms plan §6's claim that the AVG is shared across Black Widow and Space Duel.
- **`COIN65`** — coin handling, 11 titles. Crystal Castles' equivalent is `CCN.MAC`,
  which carries the same `$COINA`/`$BCCNT`/`$CMODE` symbols documented in
  `smc-gate.md`. So the shared-library pattern is archive-wide.

Note `M6502.MAC` appears **only** in crystal-castles. Its own header instructs each
project to take a private copy, so equivalents elsewhere are differently named or
absent — consistent with the per-game dialect drift the plan predicted.

## 6. The three titles the plan asks about

**Battlezone — present, and better equipped than hoped.** 104 KB, 11 `.MAC`.
Beyond the game (`BZONE.MAC`, `BZMTNS.MAC` mountains, `BZSOUN.MAC`, `BZSTST.MAC`)
and the vector macros (`VGMC`, `VGUT`), it carries the **math box**:
`MBUDOC.DOC` (documentation), `MBUCOD.COM` / `MBUCOD.MAP` / `MBUCOD.V05`
(microcode and map), and `MBDIAG.MAC` (diagnostics). Plan §6's parked option is
viable and its "math box internal documentation" is confirmed present. It has **no
HLL package**, so it would exercise the relooper path hard.

**Space Duel — present and substantial, not visibly corrupt.** 184 KB, 26 `.MAC`,
24 `.DAT`, 6 `.LDA`, plus a `revision/` subtree. Has HLL65F *and* the vector
macros. Its contents reveal it was built on Asteroids Deluxe (`AST2RD.MAC`,
`ASTRD2.MAC`, `A2*.MAC`). The plan's caution that it may be corrupt is **not
corroborated by the file listing**, but file listings cannot detect corruption —
this needs a content check before relying on it.

> **Content check done, 2026-08-11: no corruption found.** Every test was run
> against `crystal-castles` as a control, since that tree is known good — we
> assemble it byte-identically.
>
> - No empty files. No NUL bytes anywhere in content: all 6,523 of them are
>   trailing pad, and **every one of the 26 `.MAC` files is an exact 512-byte
>   multiple**. That is RT-11 block structure preserved intact, and
>   `crystal-castles` has exactly the same property (8,907 pad bytes, 96 of 96
>   files block-aligned). A damaged dump would not keep that.
> - 22 of 26 `.MAC` files end in `.END`.
> - All three `.INCLUDE` targets — `AS2DEC`, `HLL65F`, `VGMC` — resolve inside
>   the tree.
> - The 160 KB root `AST2RD.MAC` parses. Symbol names come out clean, and
>   HLL65F **expands** — the generated `~L311$100`-form labels appear in the
>   error output, which they could not if the macro package were damaged.
>
> Assembling it reports 3,379 errors, and **none of them indicate damage**: only
> 8 are parse anomalies, the rest are undefined symbols and their cascade. The
> cause is structural rather than textual — see below. On tree evidence it is the
strongest candidate for the plan's Phase 6 vector title: it has both the HLL
package (so the structured path applies) and `.LDA` oracles.

**Centipede — closer in dialect than expected in one way, further in another.**
89 KB, 24 `.MAC`, with `revision.v2/` as a second version, ROM images numbered
`136001.1xx`/`2xx` in the same scheme as Crystal Castles' `136022-1xx`, and
`CENTI.DOC`/`CENTP2.DOC` release documents. It shares `COIN65.MAC`. But it has
**no HLL package**, so plan §6's "natural third" would not benefit from the
structured-control-flow route at all.

## 7. Consequences for the plan

1. **Multi-game framing survives, and comfortably.** Twenty coin-op titles with
   real assembly source, three shared macro-library families, and one worked
   second version for several titles. Plan §8's "archive thinner than hoped" risk
   does not materialise.
2. **But the archive is two dialects, not one.** Half the titles predate HLL65.
   Any claim that structured control flow makes this tractable applies to the
   1981–83 group only. The relooper is not optional infrastructure.
3. **Plan §6's second-target recommendation should be revisited.** It suggests
   Asteroids Deluxe, Black Widow or Space Duel. On this evidence Asteroids Deluxe
   is the *weakest* of the three (no HLL package), while Space Duel and Black
   Widow both have HLL65F plus the shared vector macros. Black Widow's `VGMC16`/
   `VGUT16` differ from the `VGMC`/`VGUT` used by four other titles, so Space Duel
   sits on the more reusable branch.
3a. **Space Duel is multi-module and linked; Crystal Castles was not.** Found
   while content-checking (above), and it is the largest single piece of Phase 6
   work nobody had costed. `A2IRQ.MAC` opens with `.CSECT` and `.GLOBL`, and the
   symbols it reports undefined — `IANGLE`, `GTIME`, `TEMP4`, `INTRPT` — are not
   in `AS2DEC` or any file it includes. They are resolved **at link time**. So
   the undefined-symbol errors are correct behaviour for assembling one module
   of a linked program in isolation, not a defect in the source and not a
   dialect gap.

   Crystal Castles assembles as a single root file that includes everything, so
   `chill65-asm` never needed a linker and does not have one. Space Duel needs
   either LINKM's semantics implemented, or the modules concatenated in the
   right order with globals resolved — a decision to make deliberately rather
   than discover. This raises the priority of §7.4's advice to read the LINKM
   sources from "before deciding O1" to "before starting Phase 6".

3b. **The build recipe, and which oracle is which.** `space-duel/SDGEN1.COM` is
   the original RT-11 build script and survives intact (read it with the NULs
   stripped). MAC65 assembles 16 modules separately, then LINKM links them:

   ```
   VX0:AST2RD/L,VX0:A2LINK=VX0:AS2ROM,AST2RD,AST2RT,AS2SAC,AS2POK,AS2COI/C
   VX0:A2NAME,AS2MSG,AS2FIL,AS2TST,A2IRQ,A2EARO/C
   VX0:XYSIG,VGUTR2,A2GOOF
   VX0:A2SHIP/L=VX0:A2SHIP
   ```

   The script even prints `LINK COMPLETED, OUTPUT = 'VX0:AST2RD.LDA'`. So
   **`AST2RD.LDA` is linked from those 15 modules in that exact order**, and
   `A2SHIP.LDA` is a separate single-module link. That settles the Phase 1 target
   without having to infer it.

   `SDGEN2.COM` holds no commands: 980 bytes of leftover *assembly source
   fragments* in blocks a previous file once occupied — an RT-11 artefact, not a
   second recipe.

   **The `.LDA` oracles**, by record walk (`01 00 <count:16le> <addr:16le> …`):

   | File | Records | Covers |
   |---|---|---|
   | `AST2RD.LDA` | 1002 | `0000-8FFF` |
   | `ASTRD2.LDA` | 1002 | `0000-8FFF` |
   | `SDUEL0.LDA` | 970 | `0000-8FFF` |
   | `A2SHIP.LDA` | 55 | `2800-2FFF` |

   All three full-size images differ from one another. `revision/` duplicates are
   byte-identical to their top-level twins (`AST2RD.LDA`, `A2SHIP.LDA`,
   `AST2RD.MAC` all `cmp`-clean), so that subtree adds nothing.

   `ASTRD2.MAC` is a later patch release of `AST2RD.MAC` — 9 changed hunks,
   including the `CKUM2` checksum going `.BYTE 055` to `042` and a region fenced
   with "LEFT IN IN ORDER NOT TO HAVE TO RE-RELEASE ALL OF THE SPACE DUEL EROMS.
   THERE ARE NO PATHS TO THIS CODE" — and it pairs with `ASTRD2.LDA`. It and
   `SDUEL0.LDA` are out of scope; **the Phase 1 target is `AST2RD.LDA`**, with
   `A2SHIP.LDA` as a standalone stepping stone that needs no linker at all.

3c. **Section survey: 14 named CSECTs, not the handful expected.** Per module, in
   link order:

   | Module | Sections |
   |---|---|
   | `AS2ROM` | `.ASECT` |
   | `AST2RD` | `.ASECT`, **`MULTB`, `EXPIC`, `ROCKDAT`** |
   | `AST2RT` | `AST2RT` |
   | `AS2SAC` | `AS2SAC` |
   | `AS2POK` | `AS2POK` |
   | `AS2COI` | none at top level |
   | `A2NAME` | `A2NAME` |
   | `AS2MSG` | `AS2MSG` |
   | `AS2FIL` | `A2FILL` |
   | `AS2TST` | `.ASECT`, unnamed `.CSECT`, `AS2TST` |
   | `A2IRQ` | `AS2IRQ` |
   | `A2EARO` | `A2EARO` |
   | `XYSIG` | `.ASECT`, `XYSIG` |
   | `VGUTR2` | `VGUTR2` |
   | `A2GOOF` | none at top level |
   | `A2SHIP` | `.ASECT` only |

   Distinct named sections: `A2EARO A2FILL A2NAME AS2IRQ AS2MSG AS2POK AS2SAC
   AS2TST AST2RT EXPIC MULTB ROCKDAT VGUTR2 XYSIG`, plus one unnamed `.CSECT` in
   `AS2TST`.

   Two things here contradict a first reading and matter for the section model:
   **`AST2RD.MAC` is not `.ASECT`-only** — it carries three named sections of its
   own — and `AS2TST` uses a bare unnamed `.CSECT` distinct from its named one,
   so the model cannot treat "no operand" as "absolute". `AS2COI` and `A2GOOF`
   declare nothing at top level; what their includes establish is unchecked.

   Crystal Castles contains **no `.CSECT` at all**, verified by grep. That is the
   control fact that makes this safe to build: its output must be unchanged by
   any of it.

3d. **Triage of the 15-module assembly, and it has one dominant cause.**
   Assembling all fifteen program modules together through `assemble_units` in
   SDGEN1.COM order reports **7,805 errors**:

   | Count | Kind |
   |---|---|
   | 5,426 | undefined symbol (285 distinct) |
   | 2,026 | cascade — "requires an operand" |
   | 347 | cascade — "branch out of range" |
   | 6 | **expression ended unexpectedly** |

   The two cascade kinds are not independent: an undefined operand reports both
   `undefined symbol X` *and* `OPCODE requires an operand`, proved on a
   three-line synthetic file. So 2,373 of these are echoes.

   The undefined symbols break down, and the shape is the finding:

   | Count | Symbol | What it is |
   |---|---|---|
   | 1,887 | `...4` | `ASCIN` macro internal |
   | 993 | `...C` | `ASCIN` macro internal |
   | 525 | `~L311$90/100/130` | HLL65F generated labels |
   | 1,076 | `CNTSCL`, `VGADD2`, `VGSTAT`, `SCRCLR`, `VGRTSL` | defined **and** `.GLOBL`-exported by AST2RD.MAC |
   | rest | `TEMP1`-`TEMP9`, `VGLIST`, `HSCORE`, `ONTIME` … | also defined in AST2RD.MAC |

   **Almost all of it traces to one unimplemented directive.** `AST2RD.MAC:206`
   defines a macro `ASCIN` that converts a string to the game's own character
   codes, and its first act is `.NCHR ...C,<STRING>` — set `...C` to the
   character count. `.NCHR` does not exist in `chill65-asm`: 0 uses in Crystal
   Castles, 5 in Space Duel. `...C` is therefore never defined, `...4` never
   assigned, and the macro fails everywhere it is used. Because AST2RD.MAC is
   also where the vector-generator entry points and the scratch symbols are
   defined and exported, its failure orphans every module that imports them —
   which is why symbols that *are* present and *are* exported still read as
   undefined.
   
   That reframes the middle rows of the table: `CNTSCL` and friends are not
   missing link-time imports. They are casualties.

   **Category (b), parse and dialect, is not near zero as expected.** Two gaps:
   `.NCHR`, and the six `expression ended unexpectedly` errors, which come from
   the same macro — `...4=''...5` uses `''` as MACRO-11's character-value
   operator, and the lexer currently treats `'` only as the concatenation mark
   (`lexer.rs` leaves it as an uninterpreted `Tok::Quote` for `macros.rs` to
   decide). Five `.NCHR` uses and six `''` expressions, in one macro, account
   for the bulk of 7,805 errors.

   **Category (c), the section model, cannot yet be observed as image
   collisions** because assembly never completes. The static evidence stands
   though: eleven named `.CSECT`s plus one unnamed, and **not one of the CSECT
   modules uses an explicit `.=`**, so every one of them is purely relocatable
   and currently assembles from `loc = 0` (`assemble.rs:260`, with
   `.ASECT`/`.CSECT`/`.PSECT` all folded to a no-op at `directives.rs:66`).
   They would overwrite each other wholesale. Nothing about that changes the
   plan; it confirms the section work is necessary and not merely tidy.

   Suggested order, which differs from the plan's: `.NCHR` and the `''`
   character-value operator are worth clearing **before** the section model, on
   the evidence above — the error count should collapse by an order of
   magnitude and expose whatever is actually underneath, rather than building
   sections against a signal dominated by one macro.

3e. **`.NCHR` and the character-value operator, cleared.** Both implemented;
   the 15-module error count falls **7,805 → 4,925**, and undefined symbols
   **5,426 → 2,546**. `...4` and `...C` are gone entirely — they were the two
   largest entries and between them 2,880 errors.

   Two things had to be right, and only the first was expected:

   - `.NCHR SYM,<text>` counts the argument's **raw characters**, as `.IRPC`
     does. `<0123>` is four, not the value those digits evaluate to.
   - `'` is both MACRO-11's character-value operator and its concatenation
     mark, and the corpus uses all three combinations — `B'COND` (mark, Crystal
     Castles), `LABEL''X''Y` (two marks, AS2POK) and `...4=''...5` (operator
     then mark, AST2RD). **Whitespace does not distinguish them**: `.BYTE ''C`
     separates the operator with a space and `...4=''...5` does not. What works
     is that a quote is a mark when it either touches fusable text on its left
     or introduces a bound parameter on its right. An earlier rule based on
     spacing fused `.BYTE` with its quote and produced a mnemonic named
     `.BYTE'`.

   **What is left, and it is now the real subject of the section work:**

   | Count | Kind |
   |---|---|
   | 2,546 | undefined symbol (283 distinct) |
   | 2,026 | cascade — "requires an operand" |
   | 347 | cascade — "branch out of range" |
   | 6 | expression ended unexpectedly |

   The undefined symbols are now led by `CNTSCL` (350), `VGADD2` (195),
   `VGSTAT` (181), `SCRCLR` and `VGRTSL` (175 each) — the vector-generator
   entry points, plus the `~L311$*` HLL65F labels and the `TEMP*` scratch
   symbols. All are defined and `.GLOBL`-exported by `AST2RD.MAC`, so they
   should resolve across units; that they do not is the question §7 item 3a's
   linking work has to answer, and it is no longer masked by a macro.

3f. **ASTRD2.MAC is the better Phase 1 root — but not for the reasons first
   given here, which were wrong and are retracted.**

   An earlier version of this item claimed AST2RD.MAC was *damaged*: that a
   region of ~1,000 lines was absent from ASTRD2.MAC, that `CNTSCL`, `VGADD2`,
   `LALJSR` and `LXHJSR` appeared 175-351 times in one file and **zero** times
   in the other. **Those zeroes were an artefact of the measurement.**
   ASTRD2.MAC contains NUL bytes mid-file, so `grep` classifies it as binary and
   `grep -c` prints nothing and exits 1. The blanks were read as zeroes. Counted
   properly, on raw bytes:

   | | AST2RD.MAC | ASTRD2.MAC |
   |---|---|---|
   | `TEMP2` | 41 | **114** |
   | `CNTSCL` | 351 | **4** |
   | `LALJSR` | 175 | **18** |

   So the content is not absent from ASTRD2.MAC; it is *reduced*. The damage
   claim does not follow and is withdrawn. Two greps in this effort have now
   silently misled — this one, and reading a `grep -l` hit as a definition when
   it was a `.GLOBL` declaration. **Prefer reading these files through Python
   with explicit `latin-1` decoding; `grep` is not reliable on them.**

   What survives, each measured by the assembler or by exact byte counting
   rather than by grep:

   - Assembled alone, **AST2RD.MAC reports 3,401 errors and ASTRD2.MAC 413**.
   - **ASTRD2.MAC defines every label AST2RD.MAC defines, and 160 more** — 458
     against 298, with 298 shared and *nothing* unique to AST2RD.MAC. It is a
     strict superset.
   - **ASTRD2.MAC has an `.END`; AST2RD.MAC has none**, alone among the root
     modules (the seven other files lacking one are include files, which
     legitimately have none).
   - Both carry identical headers — `.TITLE AST2RD-ASTERIODS 2 (28503)`, same
     date, same project number — so they are two revisions of one module, and
     item 3b's finding that ASTRD2 is the later release stands.

   **Recommendation unchanged: root the build at `ASTRD2.MAC`, oracle
   `ASTRD2.LDA`.** The conclusion was right; the argument was not.

3g. **Re-triage against the intact root, and sections are still not shown to be
   the blocker.** Fifteen modules in SDGEN1.COM order with ASTRD2 substituted:

   | Baseline | Errors |
   |---|---|
   | Original, AST2RD root | 7,805 |
   | After `.IRP`/`.IRPC`/`.NCHR`/char-value | 4,925 |
   | **ASTRD2 root** | **1,895** |

   1,038 undefined symbols across 271 distinct names, 846 cascade, and — new —
   "branch out of range" has disappeared entirely, which is what one expects
   when operands start resolving. Five genuine parse anomalies remain, at
   ASTRD2.MAC:3685 (`expected an operand, found Prefix(X)`, twice at two
   columns) and :5012 (`expected an operand, found Punct(':')`).

   **A concrete new gap: `.GLOBB` is not implemented.** Nine uses in Space Duel,
   none in Crystal Castles; the assembler emits `unknown mnemonic .GLOBB` and
   counts it unhandled. It appears to declare byte-sized globals —
   `AST2RD.MAC:140` reads `.GLOBB TEMP3,UPDFLG,FRAME,TEMP2,TEMP4,...` next to
   `TEMP2: .BLKB 2`. Since the remaining undefined symbols are dominated by
   exactly those `TEMP*` names, this is the first thing to try, and it is a
   directive gap rather than a linking one.

   **Answer to whether the section model is needed: still not established.** No
   evidence yet attributes any remaining error to sections. `.GLOBB` should be
   cleared first, and the taxonomy re-measured, before any section work is
   contemplated.

3h. **The cause of the undefined symbols: exports declared with `.GLOBL` and
   defined with a *single* colon never reach the global table.** Not linking,
   not sections — a symbol-visibility bug, isolated to four synthetic lines:

   ```text
   A.MAC   .GLOBL X   /  .=0090  /  X:  .BLKB 1
   B.MAC   .GLOBL X   /  .=0A000 /  LDA X      -> undefined symbol X
   C.MAC   .GLOBL Y   /  .=0091  /  Y:: .BLKB 1
   D.MAC   .GLOBL Y   /  .=0A010 /  LDA Y      -> resolves
   ```

   In MACRO-11, `.GLOBL X` declares the symbol global and `X:` defines it; the
   two together export it. `X::` is shorthand for both at once. **Our assembler
   only honours the shorthand**, so a module that separates the declaration from
   the definition exports nothing.

   Crystal Castles writes `::` throughout, which is why this was never caught.
   Space Duel separates them everywhere — `ASTRD2.MAC` declares its scratch page
   with `.GLOBB` at lines 128-145 and defines it with plain `TEMP2: .BLKB 2` at
   277 — so almost every cross-module symbol it publishes is invisible.

   Evidence that this is the bulk of the remaining 1,038: of the twenty most
   frequent undefined names, **every one is defined, in a module that is in the
   fifteen** — `VGADD2`, `VGVTR5`, `VGCNTR` and `VGHEX` in VGUTR2; `MESGPOS` in
   AS2MSG; `TEMP1`-`TEMP9`, `TEMPA`, `TEMPC`, `VGLIST`, `HSCORE`, `ONTIME`,
   `EABUF` and `INITL` in ASTRD2. Category (a), defined nowhere, has exactly one
   member — `CHAR....X`, whose name has the shape of a macro concatenation
   artefact and which is a separate question. Category (b), orphaned by a
   failing module, is not needed to explain anything.

   This also explains why implementing `.GLOBB` alone changed nothing (item
   3g): adding a name to the declaration set cannot help when the *definition*
   never publishes it.

   **The fix, and its risk.** Export a symbol when it is defined and its name
   has been declared `.GLOBL`/`.GLOBB` in the same unit, not only when written
   `::`. The risk is that Crystal Castles may contain `.GLOBL`-declared,
   single-colon symbols that are currently private and would become visible,
   changing its import sizing — `globl_declared_externals_size_absolute` in
   assemble.rs turns on exactly that distinction. Its gate is the guard and must
   stay at DATA 16384/16384 and PROGRAM 24576/24576.

   With that fixed, `.GLOBB`'s byte-sizing (item 3g) becomes worth revisiting,
   since the two interact: `.GLOBB` says an import is zero-page, and until
   exports work there are no imports to size.

3i. **Fixed, and it was the bulk of it.** A symbol is now exported when it is
   defined *and* its name has been declared `.GLOBL`/`.GLOBB` in the same unit,
   not only when written `::` (`assemble.rs`, `define`). `.GLOBL` declarations
   are collected on both passes rather than pass one alone, because a
   `NAME = expr` definition runs on both and would otherwise be filed shared on
   the way through and private on the way back.

   | Baseline | Errors | Undefined | Distinct |
   |---|---|---|---|
   | Original, AST2RD root | 7,805 | 5,426 | 285 |
   | After `.IRP`/`.IRPC`/`.NCHR`/char-value | 4,925 | 2,546 | — |
   | ASTRD2 root | 1,895 | 1,038 | 271 |
   | **Single-colon exports** | **731** | **385** | **80** |

   **The predicted risk did not materialise.** Crystal Castles is unchanged —
   DATA 16384/16384 and PROGRAM 24576/24576, and
   `globl_declared_externals_size_absolute` still passes, because the import
   sizing turns on the *declaration* and `defined_here`, neither of which this
   touches. Space Duel's A2SHIP gate holds at 2048/2048.

   What is left is now dominated by `.GLOBB`, and cleanly: `TEMP2` (43),
   `VGLIST` (36), `TEMP1` (28), `TEMP7` (24), `TEMPA` (23) — the scratch page
   `ASTRD2.MAC:128-145` declares with `.GLOBB`, which the assembler still does
   not know, so those names never enter the declaration set and the fix above
   cannot reach them. Item 3g's directive is the next piece and its effect is
   now predictable rather than speculative. `CHAR....X` (14) remains the one
   name defined nowhere.

   **Sections are still not implicated.** Four rounds of triage and not one
   error yet attributes to them.

3j. **The program gate exists, and it settles the section question: sections
   *are* the blocker.** `gate_sd.rs` now carries
   `program_image_is_byte_identical` — fifteen modules in SDGEN1.COM order with
   ASTRD2 substituted, compared over `0000-8FFF` against `ASTRD2.LDA`. It is
   committed **failing**, at 18,870 of 36,864 bytes (51.19%), because there is
   nothing yet to make it pass; what it is for at this stage is measurement.
   The assertion is the target and is not to be weakened.

   Assembling each module alone shows where each one thinks it lives:

   | Module | Bytes | First spans |
   |---|---|---|
   | `AS2ROM` | 4,096 | `3000-3FFF` |
   | `ASTRD2` | 11,523 | `2800-2988`, `4000-4008`, … |
   | `AST2RT` | 198 | `0000-0001`, `0005-000A`, … |
   | `AS2POK` | 764 | `0000-0230`, … |
   | `XYSIG` | 277 | `0000-0114` |
   | `A2EARO` | 884 | `0000-001D`, … |
   | *(and six more)* | | all from `0000` |
   | `AS2FIL`, `A2GOOF` | 0 | nothing at all |

   **Only the two `.ASECT` modules are placed.** Every `.CSECT` module starts at
   zero and overwrites the last, exactly as item 3c predicted from the static
   survey and as item 3g could not yet observe. The oracle occupies
   `0000-0005`, `3000-8F52` and `8FFA-8FFF`; we write 2,087 addresses it never
   writes — the pile below `3000` — and miss 8,705 it does.

   **This supersedes 3g's "sections are still not shown to be the blocker".**
   They were not shown because assembly failed too early to reach the question;
   with the visibility fix in 3i the image gets far enough to answer it. The
   remaining 731 errors and the placement problem are now separable concerns,
   and placement is the larger one: it accounts for essentially all 17,994
   differing bytes, while the undefined symbols are concentrated in the
   `.GLOBB` scratch page.

   ~~`AS2FIL.MAC` and `A2GOOF.MAC` emitting zero bytes is unexplained.~~
   **Withdrawn — a measurement error, see item 3p.** Both emit correctly; the
   probe read `Assembler::image` after a clean `assemble_units`, which moves
   the image out into the `Ok` value and leaves the field empty.

3k. **`.GLOBB` implemented, and it means what the name suggests.** `.GLOBL`
   plus "and it is a byte": the symbol lives in the zero page, so a unit that
   references it without defining it sizes the operand short even though it
   cannot see the value. The *declaration* carries the size, which is the only
   reason to write `.GLOBB` rather than `.GLOBL`.

   **Why the earlier attempt (3g) failed.** It set `size_value = Some(0)` and
   left the rest alone. But `resolve_mode`'s `zp_ok` reads
   `ama && value < 0x100` — AMA here is "auto memory addressing", and *without*
   it every operand is absolute whatever its value. `VGUTR2.MAC` has no
   `.ENABL` at all and two `.GLOBB` declarations, so a rule that only widens
   what AMA already permits cannot reach it. `zp_ok` now takes a second,
   independent route: `byte_sized || (ama && …)`.

   | Baseline | Errors | Undefined | Distinct |
   |---|---|---|---|
   | ASTRD2 root | 1,895 | 1,038 | 271 |
   | Single-colon exports (3i) | 731 | 385 | 80 |
   | **`.GLOBB`** | **69** | **54** | **39** |

   Most of that drop is the *declaration* half, not the sizing half: `.GLOBB`
   now enters `global_decls`, so 3i's export rule reaches the scratch page it
   declares. The two changes only work together, which is why neither moved the
   count alone.

   Crystal Castles is unchanged (16384/16384, 24576/24576). Its single
   `.GLOBB`, at `CRP.MAC:56`, was previously producing the right bytes for the
   wrong reason — the directive was unrecognised, so the names never became
   imports and AMA sized them zero page from their values. Now they are imports
   declared byte-sized and take the same `a5 a3` for the reason the original
   assembler had. `globl_declared_externals_size_absolute` still passes and is
   the control: same symbol, same value, `.GLOBL` instead of `.GLOBB`, still
   absolute.

   What remains of the 69: `CHAR....X` (14, defined nowhere, name shaped like a
   macro concatenation artefact), a scatter of `WNDSE*`/`BOX*` singletons, and
   the 11 parse anomalies at `ASTRD2.MAC:3685` and `:5012` already recorded.

3l. **The link model, measured: named sections concatenate in link order.**
   With operand widths correct, each relocatable module can be assembled alone
   and located in `ASTRD2.LDA` by searching for its longest byte runs. Before
   `.GLOBB` this drifted — `AST2RT`'s two runs implied bases one byte apart,
   which is exactly an operand emitted absolute where the original emitted zero
   page. Now the anchors agree with themselves:

   | Module (link order) | Implied base | Agreeing runs |
   |---|---|---|
   | `AS2ROM` | `3000` (`.ASECT`) | absolute |
   | `ASTRD2` | `2800`, `4000`+ (`.ASECT`) | absolute |
   | `AST2RT` | `6EE5` | 2, still 3 bytes apart |
   | `AS2SAC` | `703C` | 2 |
   | `AS2POK` | `70C0` | 2 |
   | `AS2COI` | `741A` | 2 |
   | `AS2MSG` | `7730` | 1 |
   | `AS2TST` | `7FAB` | 1 |
   | `A2IRQ` | `8639` | **4** |
   | `VGUTR2` | `8E42` | 1 |

   **The bases rise monotonically in `SDGEN1.COM`'s link order.** That is
   concatenation, and it is measured rather than assumed — `A2IRQ` alone has
   four independent runs agreeing on one base. `A2NAME`, `A2EARO` and `XYSIG`
   did not anchor; `AS2FIL` and `A2GOOF` still emit nothing at all (item 3j).

   `AST2RT`'s two runs remain 3 bytes apart, so something inside that module
   still sizes differently from the original. That is a real remaining
   difference and should be run down rather than absorbed into a base guess.

3m. **Sections built, and the origin is derived rather than chosen.**
   `.CSECT`/`.PSECT` are now distinct from `.ASECT`: each named section has its
   own location counter, offsets carry **across units** so two modules
   contributing to one section concatenate, and a bare `.CSECT` is a section in
   its own right rather than a synonym for absolute.

   Placement needs sizes, and sizes need an assembly, so any build containing a
   section runs twice: a throwaway probe on a fresh assembler measures every
   section at a provisional base, then the real run assembles against the
   layout. The provisional base only has to sit above `0x100` — sizes come from
   offsets, and the one thing a base can influence is whether an operand looks
   zero-page, which it does not at either the probe's base or the real ones.

   **The origin, `6D5C`, is measured.** Running the probe's sizes back from
   each oracle anchor gives the origin that anchor implies, and the first three
   agree exactly:

   | Section | Anchor | Cumulative before it | Implied origin |
   |---|---|---|---|
   | `AST2RT` | `6EE5` | `0189` | **`6D5C`** |
   | `AS2SAC` | `703C` | `02E0` | **`6D5C`** |
   | `AS2POK` | `70C0` | `0364` | **`6D5C`** |
   | `AS2MSG` | `7730` | `08C5` | `6E6B` |
   | `AS2TST` | `7FAB` | `0F19` | `7092` |
   | `AS2IRQ` | `8639` | `2200` | `6439` |

   It also **predicts** `A2NAME` at `741A`, which the oracle confirms
   independently. Anchors further down imply other origins, but an origin is
   only as good as every size preceding it — those are accumulated size errors,
   not disagreement about where the region begins.

   The program gate moves **51.19% → 56.37%** (16,082 differing bytes, down
   from 17,994). Crystal Castles is untouched at 16384/16384 and 24576/24576 —
   it contains no `.CSECT`, so the probe never runs on it.

3n. **Next lead: the blank section may be the default, not absolute.**
   `AS2COI` and `A2GOOF` declare no section at all, in themselves or in
   anything they include, so today they assemble absolute from `0000` — and the
   gate's first difference is still at `0000`, where the oracle has nothing.
   But `AS2COI` anchors at `741A` in the oracle, on two agreeing runs.

   In MACRO-11 the default at the start of a unit is the **blank section**, not
   the absolute one; `.ASECT` is an explicit switch into absolute. If that holds
   here, `AS2COI` and `A2GOOF` belong in the same blank section that
   `AS2TST.MAC`'s bare `.CSECT` opens, which would place them rather than pile
   them at zero.

   **The risk is exactly what makes it worth testing rather than assuming.**
   Crystal Castles has no `.CSECT`, but if any of its modules also lack
   `.ASECT`, defaulting to the blank section would move their content. Its gate
   decides, as it did for 3i and 3k.

   Also still open from 3j: `AS2FIL` and `A2GOOF` emit **zero bytes**, and
   `AST2RT`'s two anchors remain 3 bytes apart.

3o. **The blank-section rule confirmed, and the remaining work is section
   sizes.** 3n's hypothesis holds: a unit begins in the blank section, not the
   absolute one. `AS2COI.MAC` and `A2GOOF.MAC` declare no section anywhere, and
   the oracle puts `AS2COI` at `741A` — exactly where `AS2POK` ends. Crystal
   Castles is unaffected because every root declares `.ASECT` before emitting
   anything.

   That also forced a second rule: **a section joins the layout when content
   lands in it**, not when a directive naming it goes by. Every unit now starts
   in the blank section, so registering on entry would put the blank one first
   in every build.

   The gate moves **56.37% → 57.99%**, and the first difference leaves `0000`
   altogether — nothing is piled at zero any more.

   **Three bases are now exact**, and the errors past them are size errors, not
   placement errors:

   | Section | Our base | Anchor | Out by |
   |---|---|---|---|
   | `AST2RT` | `6EE5` | `6EE5` | **0** |
   | `AS2SAC` | `703C` | `703C` | **0** |
   | `AS2POK` | `70C0` | `70C0` | **0** |
   | `~blank` | `741A` | `741A` (as `AS2COI`) | **0** |
   | `AS2MSG` | `774B` | `7730` | +`1B` |
   | `AS2TST` | `7D9F` | `7FAB` | −`20C` |
   | `AS2IRQ` | `8D9F` | `8639` | +`766` |
   | `VGUTR2` | `95A8` | `8E42` | +`766` |

   Read down the column: the layout is right through `~blank`, then `A2NAME` is
   **27 bytes too large**, `A2FILL` or `AS2MSG` is **~0x227 too small**, and
   `AS2TST` is **~0x972 too large**. `AS2IRQ` and `VGUTR2` are out by the same
   `766`, which says nothing new goes wrong between them — the error is fully
   accumulated by `AS2IRQ`.

   **Remaining gate state: 21,377 of 36,864 (57.99%), 724 differing runs**,
   nearly all one or two bytes at operand positions — addresses pointing into
   sections whose bases are still off. That is the expected shape when sizes
   are wrong and placement is right, and it is the next piece of work: run down
   `A2NAME`, `A2FILL`/`AS2MSG` and `AS2TST` one at a time, each against its own
   anchor. Also still open: `AS2FIL` and `A2GOOF` emit zero bytes (3j), and
   `AST2RT`'s two anchors sit 3 bytes apart (3l).

3p. **Three more rules, all measured, and the layout is now exact through
   `AS2MSG`.** Chasing one instruction — `COIN65.MAC`'s `ADC $CNCT`, which the
   original assembles `65 26` (zero page) where we emitted `6d 26 00` — turned
   up two corrections, and locating `A2GOOF`'s own bytes turned up a third.

   1. **Being an import is a whole-unit fact.** `COIN65.MAC` uses several names
      well before the blocks that define them, and consulting `defined_here` at
      the point of use called them imports and sized them absolute. The section
      probe now runs for **every** build, not only those with sections, and
      hands back what each unit defines anywhere.

   2. **`.GLOBB` is not scoped to the unit that writes it.** It says the symbol
      *is a byte* — a fact about the symbol, which the linker carries across the
      program. `$CNCT` is declared `.GLOBB` in `ASTRD2.MAC` and used unprefixed
      by `COIN65.MAC`, which declares only `.GLOBL` for it. Crystal Castles is
      the control and stays exact: its one `.GLOBB` names `$INTCT` and
      `ATRACT`, while `SN.NUM`, declared only `.GLOBL`, stays absolute.

      This forced the `.GLOBB` test's control to be **rewritten rather than
      kept**. It had asserted per-unit scoping by asking one unit to treat a
      byte symbol as wide, which under a build-wide rule asks the linker to
      hold two answers at once. It now uses a second symbol nothing declares
      `.GLOBB`, and additionally asserts byte-ness reaching a unit that never
      declared it.

   3. **The blank section belongs to a unit, not to the program.** A named
      `.CSECT` is shared; the blank one is not. The oracle puts `AS2COI`'s blank
      content at `741A` and `A2GOOF`'s at `8F43` — two and a half kilobytes
      apart, each immediately behind whatever precedes it in link order.
      Merging them dropped `A2GOOF`'s sixteen bytes at `741A` as well and
      pushed every section after it sixteen bytes late.

   **Correction to 3j.** `AS2FIL` and `A2GOOF` do *not* emit zero bytes. The
   probe read `Assembler::image` after a clean `assemble_units`, which
   `mem::take`s the image into the `Ok` value. `A2GOOF` emits its sixteen bytes
   — `78 8d 0b 10 ...`, the code that had been sitting at `0000` — and `AS2FIL`
   emits 807 into `A2FILL`. `AS2FIL`'s `.REPT 327` is also correct: `.RADIX 16`
   is in force, so it is 807, not 327.

   **The gate: 51.19% -> 60.16%** across these. Crystal Castles never moved.

3q. **What is left, and how it was measured.** Anchoring must use the **full
   build's** bytes, not a module assembled alone — a standalone module's
   internal offsets are not the ones it ships with, and the earlier `AS2TST`
   and `AS2IRQ` anchors were computed that way and are unreliable. Re-anchored
   against the full build:

   | Section | Our base | True base | Out by |
   |---|---|---|---|
   | `AST2RT` | `6EE5` | `6EE5` | **0** |
   | `AS2SAC` | `703C` | `703C` | **0** |
   | `~blank@AS2COI` | `741A` | `741A` | **0** |
   | `~blank@A2GOOF` | `968B` | `8F43` | -1864 |

   So the layout is exact through the blank section, and by the end of the
   image our cumulative is **1,864 bytes too large**. Working backwards from
   `A2GOOF` at `8F43`: `VGUTR2` `8E42`, `XYSIG` `8D33`, `A2EARO` `874A`,
   `AS2IRQ` `863C`. Forward from `A2FILL`'s end at `7D84`, that makes
   **`AS2TST`'s section 2,232 bytes, where we measure 4,732** — it is the
   dominant remaining error by a wide margin, and everything after it is
   carried along.

3r. **The layout re-anchored, and 3q's arithmetic corrected.** With `.VCTRS`,
   the delimited-argument and the export fixes in, the section chain is exact
   from the origin all the way to `AS2IRQ`:

   | Section | Our base | Size | Oracle | Delta |
   |---|---|---|---|---|
   | `MULTB` | `6D5C` | `0100` | — | — |
   | `EXPIC` | `6E5C` | `0020` | — | — |
   | `ROCKDAT` | `6E7C` | `0069` | — | — |
   | `AST2RT` | `6EE5` | `0157` | `6EE5` | **0** |
   | `AS2SAC` | `703C` | `0084` | `703C` | **0** |
   | `AS2POK` | `70C0` | `035A` | — | — |
   | `~blank@AS2COI` | `741A` | `010F` | `741A` | **0** |
   | `A2NAME` | `7529` | `0207` | — | — |
   | `AS2MSG` | `7730` | `05D5` | `7730` | **0** |
   | `A2FILL` | `7D05` | `0327` | `7D05` | **0** |
   | `AS2TST` | `802C` | `060D` | `802C` | **0** |
   | `AS2IRQ` | `8639` | `010E` | `8639` | **0** |
   | `A2EARO` | `8747` | `05E9` | `8747` | **0** |
   | `XYSIG` | `8D30` | `010F` | — | −3 |
   | `VGUTR2` | `8E3F` | `0101` | `8E42` | −3 |
   | `~blank@A2GOOF` | `8F40` | `0010` | `8F43` | −3 |

   **Correction to 3q.** That item claimed `AS2TST`'s section had to be 2,232
   bytes. It does not: the claim assumed `A2FILL` ends at `7D84`, and the
   missing bytes were `AS2MSG`'s, not `AS2TST`'s. `AS2TST` is `060D` at `802C`,
   and `AS2MSG` is `05D5` — both now exact. The 1,864-byte overshoot 3q
   reported is gone; what is left is 3 bytes.

3s. **The last 3 bytes, located exactly — and they are an over-shrink, not a
   shortfall.** Walking our image from `AS2IRQ`'s base and resynchronising
   against the oracle: 776 bytes match, then one byte is missing at `8952`, two
   more at `896B`, and the remaining **1,396 bytes match to the end**. Both
   sites are inside `A2EARO`.

   The second site is `A2EARO.MAC:405-408`, `LDA GAME / ASL / CLC / ADC GAME`.
   We emit `a5 34 ... 65 34` — zero page. The original emitted
   `ad 34 00 ... 6d 34 00` — absolute, for a symbol whose value *is* `0034`.
   So this is the reverse of every sizing bug so far: we are shrinking an
   operand the original deliberately left wide.

   **This is a genuine counter-example to item 3p's build-wide `.GLOBB`.**
   `A2EARO` declares `.GLOBL GAME`; `A2NAME` declares `.GLOBB GAME`. Under a
   build-wide rule `GAME` is byte-sized everywhere, which is what makes us
   shrink it — and it is exactly the shape of the `$CNCT` case that motivated
   the rule, where `COIN65` declares `.GLOBL $CNCT`, `ASTRD2` declares
   `.GLOBB $CNCT`, and the original *did* size it zero page. In both cases the
   `.GLOBB` module precedes the using module in link order, so ordering does
   not separate them.

   **But per-unit `.GLOBB` is not the answer either — that was measured, not
   assumed.** Restoring the per-unit rule takes the gate from **71.74% to
   60.00%**. Build-wide is right in aggregate and `GAME` is a real exception,
   so something narrower than either rule distinguishes the two. Left for the
   census (item to come); not guessed at here.

4. **The toolchain source changes the O1 calculus** — see §4. Task #5 should read
   `atari_tools/OPC65.MAC` and the LINKM sources before deciding.

## 8. Unclassified and incomplete

- **`mk2`** — tree fetch returned an API error; not classified. Midway Mortal
  Kombat 2 by name, so outside scope regardless, but recorded rather than dropped.
- Classification is **by filename and extension only**. No file contents were
  fetched. Every dialect claim above is therefore an inference from names —
  strong for `HLL65F.MAC` (a distinctive name), weaker for the absence of a
  package, since a differently-named equivalent would be missed.
- Content classes "complete buildable assembly" vs "assembly with gaps" were **not
  determined**. Establishing that requires the include-closure analysis used in
  `integrity.md` §4, which needs file contents. The Crystal Castles case shows
  why this matters: its tree looks complete but `CJTB.MAC` is missing from two of
  three versions. **Assume nothing about buildability from this survey.**
- Repo sizes are GitHub's reported values, which include git overhead.
