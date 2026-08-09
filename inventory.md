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
this needs a content check before relying on it. On tree evidence it is the
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
