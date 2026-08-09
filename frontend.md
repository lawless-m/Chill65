# O1 — Rust front end versus AT6502 reuse

## Recommendation

**Reimplement in Rust.** Use Atari's own toolchain sources as the dialect
specification, and keep AT6502 at arm's length as an optional external
cross-check rather than a component.

The plan deferred this decision until archive dialect variation was known
(plan §7, O1). Task #4 supplied that, and also turned up something the plan did
not anticipate: **Atari's own assembler and linker are in the archive in source
form**. That changes the question from "reimplement or reuse a reimplementation"
to "reimplement from the authoritative specification, or reuse someone else's
partial reimplementation". Put that way it is not close.

---

## 1. What AT6502 actually is

`github.com/danlb2000/AT6502` @ depth-1 clone, inspected directly.

- **3,697 lines of C#** across `Assembler.cs`, `Macro.cs`, `Opcodes6502.cs`,
  `SymbolTable.cs`, `Repeat.cs`, `AssemblerStack.cs`, plus a GOLD Parser grammar
  (`6502ASM.grm`, 131 lines) and its compiled table (`6502ASM.egt`). Has a unit
  test project.
- **Licence: GPL v3**, declared in `Readme.txt` (there is no `LICENSE` file, which
  is easy to misread as unlicensed — it is not).
- **Requires .NET Framework 4.6.1**; the README states "Windows XP or greater".

### It does not build here

```
error MSB3644: The reference assemblies for .NETFramework,Version=v4.6.1
were not found.
```

`dotnet` 9.0.308 is installed, but AT6502 targets .NET *Framework*, not .NET
Core/5+. Running it on this Linux machine needs either the Framework targeting
pack, Mono, or a retarget of the project files. Not insurmountable, but it is
friction on every build for the life of the project.

### Its dialect coverage is narrower than our corpus

Directive coverage, grepped across all `.cs` and the grammar:

| Covered | Absent |
|---|---|
| `.ASECT` `.RADIX` `.IIF` `.REPT` `.ENDR` `.MACRO` `.ENDM` `.INCLUDE` `.BLKB` `.GLOBL` `.ENABL` `.NOCROSS` `.DEFSTACK` `.PUSH` `.POP` `.PRINT` `.ERROR` `.SBTTL` `.TITLE` `.PAGE` `.NLIST` `.IFF` `.IFT` `.ENDC` | **`AMA`** · **`M68`** · **`.DSABL`** · `.BLKW` |

The two in bold are not cosmetic:

- **`AMA`** is auto zero-page management. `CSTART.MAC` and `CRP.MAC` both enable
  it (`.ENABL AMA`, commented "ENABLE AUTO ZERO PAGE MANAGEMENT"). It decides
  whether an operand assembles as 2 bytes or 3, so it moves **every subsequent
  address**. A front end that ignores AMA cannot be byte-accurate on this corpus.
- **`M68`** flips `.WORD` to 6800 byte order. `CRP.MAC`'s `LDAH` macro depends on
  it to extract the high byte of a symbol address (`smc-gate.md` §6). Without it,
  the `STRET-1` return-address synthesis in `STUNE` assembles wrong.

### Its build coverage is a subset of the real build

`CommandLines.txt`, in full:

```
CRF.MAC -LCCASTLES.ASM -Occastles.bin
C99.MAC -LC99.ASM -Ssymbols.txt -Occastles.bin
```

Only `CRF.MAC` and `C99.MAC`. But `integrity.md` §3 established from Atari's own
release documents that the program is **four assemblies and two links**
(`A CRF | A C99 | A CRP | A CLS`, then `L CRF,CRP,CLS`), and `CRF.MAC`'s twenty
`.INCLUDE` lines reach neither `CRP.MAC` nor `CLS.MAC`. Those two contribute real
content to the linked image — `CRP.MAC:474` places code at `.= 0CCE0` and
`CLS.MAC:6` at `.=0D540`, and the extracted `CRF.LDA` has the sound jump table
sitting at `CCE0` (`4c d4 cd 4c b0 cd …`).

So AT6502 as configured cannot be reproducing the whole ROM.

**This qualifies a plan §2 "established fact."** The plan records "AT6502
reassembles the source to the MAME `ccastles1` ROM set byte-for-byte" and treats
it as a correctness oracle. On the evidence in the repository that claim needs
narrowing — at minimum it excludes the sound package. I have not run AT6502, so I
cannot say what it does produce; I can only say its own command lines and its
missing directives are inconsistent with reproducing the full linked image.
Recorded as a correction to be verified, not as a refutation.

It also targets **version 1**, while `integrity.md` §6 selected **version-3** —
and version 1 cannot be built at all without reconstructing `CJTB.MAC`.

## 2. What the archive gives us instead

`historicalsource/atari-coin-op-assembler`, under `atari_tools/`, holds Atari's
development tools in source, in two generations:

**`e2_tools/`** — `OPC65.MAC` + `.OBJ` (6502 opcode processor), `OPC68.MAC` +
`.OBJ` (6800), `LINK0.MAC` + `LINK0.OBJ`, `LNKV2B.OBJ`, `LNKLNK.CTL`,
`EDIT.MAC`, `PIP1.MAC`, `BATCH.MAC`, `ODT.OBJ`, `BA.SYS`.

**`e3_tools/`** — `MAC69.MAC` (macro assembler), `PST65.MAC`/`PST68.MAC`/
`PST69.MAC` + `.OBJ` (permanent symbol tables — the opcode and directive
definitions), `LNKOV1–4.MAC` + `.OBJ` (the LINKM linker in overlays),
`A3MC.M65`, `M6TA.M69`, `PIP.MAC`, `FX.MAC`, `RTEXE5/8/9.MAC`.

These are written in PDP-11 assembly (`JSR PC,S`, `RTS PC`, `MOV S,-(6)`,
`.RAD50`, `.MCALL .REGDEF`) — the tools ran on a PDP-11/VAX host, which is why
the game sources are MACRO-11 shaped.

### The addressing-mode set, from the horse's mouth

`OPC65.MAC` defines the dialect's addressing modes as a bit set:

```
AMCHR:
	AMM	<<<NX >,<Z  >,<I  >,<A  >,<NY >,<ZX >>>
	AMM	<<<AY >,<AX >,<S  >,<AC >,<N  >,<ZY >>>
	AMM	<<<X  >,<Y  >>>
	.WORD	0		;STOPPER
	AM.SPC=AM.X
```

Fourteen modes: `NX` `Z` `I` `A` `NY` `ZX` `AY` `AX` `S` `AC` `N` `ZY` `X` `Y`.

**This closes the open question in task #11.** `AY,` — seen as `INC AY,$BCCNT` in
`CCN.MAC` and flagged as a halt condition in `LOOP.md` — is absolute,Y. It was
never a mystery to be guessed at; it is item seven in Atari's own table. `X` and
`Y` are the auto-sizing forms used pervasively in the game sources
(`LDA X,VITAB`), resolved to zero-page or absolute by AMA.

`OPC65.MAC` also declares `ED.AMA` among its globals and branches on `AM.S` for
branch instructions and `AM.AC` for accumulator-implied operands — so the AMA
logic AT6502 lacks is specified here in full.

## 3. The two-language cost, honestly

Reuse is not free even where it works:

- The toolchain becomes **GPL v3 by linkage**. Plan §9 wants the toolchain to be
  the publishable artefact. GPL v3 is publishable, but it is a licence Matt should
  choose deliberately, not inherit from a dependency.
- Rust-to-C# interop across a compiler boundary means either shelling out and
  parsing text, or a serialisation format — either way the IR (plan §3, "the
  load-bearing design decision") would be constrained by what C# emits.
- A .NET Framework dependency that does not build on the development machine.

And reuse buys least where the work is hardest. The expensive parts of this front
end are the macro engine (task #10 — HLL65F's `?` generated labels, `'`
concatenation, `.DEFSTACK`/`.PUSH`/`.POP`, nested expansion) and byte-exact
encoding under AMA. AT6502 implements the former and **not** the latter.

## 4. What a Rust reimplementation must cover

Already enumerated across tasks #7–#12 and the existing documents, and now
checkable against `PST65.MAC` rather than inferred:

- MACRO-11 lexing with NUL block padding, CRLF line endings, `.DAT` files with
  no line terminators at all,
  `.REPT 0` documentation blocks, trailing-dot decimals, `==` global equates.
- Expressions with `^H`/`^D`/`^B`/`^O` radix overrides, `<>` grouping, `.AND.`/
  `.OR.`/`.NOT.`, `&`, `.` as location counter, forward references.
- Directives including `.=` backward motion, `.ENABL AMA`/`M68`, conditionals,
  `.REPT`, `.ERROR`.
- Macro expansion, including the HLL65F structured-control-flow package.
- 6502 encoding across the fourteen addressing modes above, with AMA sizing.
- A LINKM-equivalent link step (task #12), whose original source is in
  `e3_tools/LNKOV1–4.MAC`.

The dialect surface is large but now **specified**, not guessed. That is the
difference this survey made.

## 5. Where AT6502 still earns its place

Not as a component, but:

- Its GOLD grammar (`6502ASM.grm`, 131 lines) is a compact independent reading of
  the syntax, useful for sanity-checking ours.
- If it can be made to run, its output on version 1 is a **second oracle**
  alongside the `.LDA` images — independent implementations agreeing is stronger
  than either alone.

Keep it as an external executable compared against, never linked. That also keeps
the GPL question at arm's length: comparing outputs is not derivation.

## 6. Consequences

- **Tasks #6–#13 stand as written.** The Rust assumption they were built on is
  the recommended one; the loop proceeds.
- **Task #11's halt risk is removed** — `AY,` is answered, and the full mode list
  is available. Task updated.
- **Task #12's linker** has an authoritative reference in `e3_tools/LNKOV1–4.MAC`.
- **Plan §2 needs a correction** — see §1, on the AT6502 oracle claim.
- Neither `atari-coin-op-assembler` nor `AT6502` may be committed to this
  repository; both are third-party. Task #6's `.gitignore` must exclude them
  alongside `crystal-castles/` and the MiSTer core.

## 7. Unverified

- **AT6502 was never run.** Everything above is from reading its source, its
  command lines and its build failure. The claim that it reproduces `ccastles1`
  byte-for-byte is neither confirmed nor refuted here.
- `PST65.MAC` and `MAC69.MAC` were identified by name and role but **not read in
  detail**. The claim that they specify the directive set follows from MACRO-11
  naming convention (PST = permanent symbol table) and from `OPC65.MAC`'s
  structure — it should be confirmed when task #9 needs the directive list.
- The `e2_tools`/`e3_tools` split is assumed to be two toolchain generations on
  filename evidence; which one built Crystal Castles in 1983 is not established.
  Its `.DOC` files say "Assembler used: MAC65", and `e3_tools` contains `MAC69`
  and `.M65` files, so the mapping is suggestive but unproven.
