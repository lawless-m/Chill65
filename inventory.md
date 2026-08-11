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
