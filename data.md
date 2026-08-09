# Phase 0.4 — castle ("city") data format

How the binary `.DAT` playfield data is laid out and consumed at run time.

Sources read: `C99.MAC`, `C00.DAT`, `CRF.MAC` (root tree — the chosen data source
per `integrity.md` §6); `CDB.MAC`, `CCT.MAC`, `CWV.MAC`, `CG.MAC` (version-3 — the
chosen program source). `CDB.MAC` is byte-identical between root and version-3.

**Radix warning.** The `.DAT` files are included under `.RADIX 10`; everything else
here is `.RADIX 16`. So `CTRAM: .BLKB 16*16` in `CG.MAC` is **22×22 = 484 bytes**,
not 256. Every dimension below is decimal; misreading this makes the whole format
incoherent, and it is the single easiest mistake to make in this data path.

---

## 1. The sixteen blocks

`C99.MAC` assembles all sixteen playfields into one 16K image at `WV.STR` = `A000`,
each occupying `WV.SIZ` = `0x400` (1024) bytes. 16 × 1024 = 16,384 = `A000`–`DFFF`,
which is exactly the extent of `C99.LDA` measured in task #1 and exactly the bank-1
data window in `hardware.md` §3.

The files are **interleaved, not sequential**. From the `.=` directives in `C99.MAC`:

| Block | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| File | C00 | C10 | C20 | C30 | C01 | C11 | C21 | C31 | C02 | C12 | C22 | C32 | C03 | C13 | C23 | C33 |

That is, file `Cxy` lands at **block index `y*4 + x`**.

Each block is padded out to its 1024 bytes with `0FF` via
`.REPT n*WV.SIZ+WV.STR-. / .BYTE 0FF / .ENDR`. Some blocks carry a checksum byte
before the padding (`C99.MAC` `.BYTE CHK04` after `C00.DAT`, `.BYTE CHK05` after
`C02.DAT`); see Unresolved.

## 2. Layout within one 1024-byte block

`C00.DAT` contains 991 defined bytes. `CRF.MAC:14-15` names the offsets:

```
EW.NUM = 16*16*2+WV.STR    ; 22*22*2 = 968  -> A3C8
EW.BLK = EW.NUM+1          ;                -> A3C9
```

| Offset | Size | Contents |
|---|---|---|
| 0 – 483 | 484 | **Height grid** — 22 × 22 bytes, loaded to `CTRAM` |
| 484 – 967 | 484 | **Attribute grid** — 22 × 22 bytes, loaded to `CTRA2` |
| 968 | 1 | **Elevator count** (`EW.NUM`) |
| 969 … | 11 × count | **Elevator records** (`EW.BLK`), `EC.BSZ` = 11 bytes each |
| … – 1023 | — | `0FF` padding, plus an optional checksum byte |

991 = 484 + 484 + 1 + (2 × 11), and `C00.DAT`'s elevator count byte is indeed `2`.

### Height grid (`CTRAM`)

Values observed in `C00.DAT`: 0, 4, 20, 30, 40, 60, 80 (decimal). Used directly as
a height in `CR.DRW` (`LDA BL.HEI / SUB TEMP1`) to decide how tall the visible
side faces of each isometric block are. Row 0 and row 21 are all zero, and the
interior is ringed with 4 — a border frame.

### Attribute grid (`CTRA2`)

Values observed: 0, 4, 5, 68, 69, 70, 196, 212, 213, 214. Read in `CR.DRW` via
`CT.A2L` and used two ways:

- passed whole to `CL.PR` — colour/priority selection;
- `AND #4` tested for what the source comments call **accessibility**, selecting
  `FC.BV3` (a bit-map face value from `CG.MAC`'s `FC.BV*` block).

### Elevator records

`EC.BSZ` is computed in `CG.MAC:246-257` as `.-EL.MOD` over this run of `.BLKB`s,
giving **11 bytes**:

| Field | Size | C00 elevator 1 | C00 elevator 2 |
|---|---|---|---|
| `EL.MOD` | 1 | 0 | 0 |
| `EL.SIT` | 1 | 40 | 40 |
| `EL.UPF` | 1 | 0 | 0 |
| `EL.MAT` | 2 | 145, 129 | 128, 128 |
| `EL.TOP` | 1 | 30 | 30 |
| `EL.BOT` | 1 | 20 | 20 |
| `EL.HP` | 1 | 56 | 212 |
| `EL.VP` | 1 | 210 | 184 |
| `EL.WAI` | 1 | 30 | 40 |
| `EL.PRI` | 1 | 1 | 1 |

`EC.USZ` = 3 (`EL.MOD`..`EL.UPF`) is a smaller "update buffer size" used elsewhere.
`EC.MAX` = 5 caps elevators per wave; `EC.BLK: .BLKB EC.BSZ*EC.MAX` is the RAM
destination.

`CG.MAC:474` carries a warning worth honouring: *"if this gets moved, change the
FORTRAN program ( elevators !!)"* — the elevator data was produced by a separate
FORTRAN tool, now lost.

## 3. Which block for which wave

`CWV.MAC` `DF.UPD`:

```
LDA WV.XCO
ASLS 2              ; *4
ADD WV.YCO          ; index = WV.XCO*4 + WV.YCO
TAX
LDA WV.TAB(X)
TAY
AND #0F             ; low nibble -> WV.NUM, the playfield 0-15
STA WV.NUM
...
TYA
AND #30             ; bits 4-5 -> region flags (CT.HR1/HR2/HR3)
```

then `WV.CMP` (`CWV.MAC:126-130`):

```
;  compute WV.OFF,  this assumes WV.SIZ=400
TRAI 0 WV.OFF
LDA WV.NUM
ASLS 2
STA WV.OFF+1        ; WV.OFF = (WV.NUM << 2) << 8 = WV.NUM * 0x400
```

So the player's map position `(WV.XCO, WV.YCO)` indexes the 37-byte `WV.TAB`
(`CWV.MAC:120`), whose entry's **low nibble selects one of the sixteen playfields**
and whose bits 4–5 carry difficulty/region flags. `WV.OFF` is then simply
`WV.NUM × WV.SIZ`, added to every ROM pointer into the data bank.

Sixteen playfields are therefore reused across a larger number of waves, varied by
the region flags.

## 4. Transfer from ROM to RAM (`CDB.MAC`)

`CT.TRA` copies a whole block out of the banked data ROM into RAM before drawing:

```
CT.TRA:
	TRAI 0FF HW.BSL          ;  select bank 1
	TR16AI CTROM,CT.ROM
	AD16AM CT.ROM,WV.OFF     ;  += wave offset
	TR16AI CTRAM,CT.RAM
	LDX #03
	LDY #00
10$:	TRAM @CT.ROM(Y) @CT.RAM(Y)
	DEY
	BNE 10$
20$:	INC CT.ROM+1
	INC CT.RAM+1
	DEX
	BMI 50$
	BNE 10$
	LDY I,2*16*16-300        ;  968 - 768 = 200
	BNE 10$
50$:	DEC CT.ROM+1             ;  transfer last byte
	DEC CT.RAM+1
	TRAM @CT.ROM(Y) @CT.RAM(Y)
```

Three full pages (768) plus 200 plus the single byte handled at `50$` = **968
bytes**, i.e. both grids, into `CTRAM` and `CTRA2` contiguously. The `50$` tail
exists because the `DEY/BNE` loop never executes at `Y=0`.

`EL.INI` immediately follows in the same bank-1 window, copying `EC.BSZ` bytes per
elevator from `EW.BLK+WV.OFF` into `EC.BLK`, with the count read from
`EW.NUM+WV.OFF`. Only at the end of `EL.INI` does `TRAI 0 HW.BSL` restore bank 0.

**This is the span that makes the interrupt bank-sniff in `hardware.md` §4
load-bearing**: an IRQ arriving anywhere between `CDB.MAC:7` and `CDB.MAC:72` finds
bank 1 mapped, and must detect and restore it. `CDB.MAC`'s own header comment —
*"this file must sit above 0E000"* — is the other half of the arrangement: the
routine has to live in the unbanked ROM at `E000`–`FFFF` so it does not vanish
when it switches the window under itself.

## 5. Drawing traversal (`CCT.MAC`)

```
CT.DRW:
	TR16AI CTRAM CT.ACL
	TR16AI CTRAM+1 CT.ARL
	TR16AI CTRAM+2 CT.AFL
	TR16AI CTRAM+CT.YDM+3 CT.ALL     ; CT.YDM=13 hex = 19; +3 = 22 = row stride
	JSR CR.INI
10$:	JSR CR.DRW
	DEC CT.CNT
	BMI 20$
	JSR CR.ADV
	JSR MN.HOU
	JMP 10$
```

Four cursors are set up at offsets 0, +1, +2 and +22 from the grid base, then rows
are drawn by `CR.DRW`, advanced by `CR.ADV`, counted down in `CT.CNT`. `MN.HOU`
("housekeeping") is called once per row — the draw is spread across the frame
rather than done atomically.

`CR.DRW` walks the blocks of one row (`BL.INI`, then `BL.IN2` per block) and for
each block reads two neighbours to decide the visible faces:

- `CT.ADL + 1` — *"square before ADL X dir"*
- `CT.ADL + 16` — *"square before ADL Y dir"*, i.e. **+22, one row down**

For each neighbour it computes `BL.HEI - neighbour_height`; if negative the face is
hidden (`BL.V1N`/`BL.V2N` set to 0), otherwise the exposed face height and start
are stored in `BL.V1S`/`BL.V1N` and `BL.V2S`/`BL.V2N`. This is classic isometric
occlusion: draw a side face only where this block stands proud of its neighbour.

The row stride of 22 is confirmed twice over — once by `CT.YDM+3` and once by the
literal `16` (hex) in the Y-direction neighbour computation.

## 6. Consequences for the project

- The data path needs no reverse engineering: block index, grid geometry, elevator
  records and the wave→block mapping are all recoverable from source.
- The runtime's bank-1 read path must be correct across the whole `CT.TRA`/`EL.INI`
  span, and `CDB.MAC` must be placed above `E000` — a placement constraint the
  emitter must not silently break.
- Open question **O5** (a standalone level viewer) is now cheap: everything needed
  to render a castle statically is documented above. Deliberately not built — out
  of scope for this task.

## 7. Unresolved

- **Attribute-grid bit semantics.** Only bit 2 (`AND #4`, "accessibility") is
  pinned. The meaning of the observed values 68/69/70, 196, 212/213/214 and the
  `CL.PR` priority scheme is not established; `CL.PR` was not read.
- **The four `CT.A*L` cursors.** Offsets 0/+1/+2/+22 are certain, their individual
  roles (`ACL`/`ARL`/`AFL`/`ALL`) are inferred from names only, not verified.
- **`CHK04` / `CHK05`.** `C99.MAC` emits these single bytes after some castles but
  not others. Presumably per-block checksums consumed by the self-test in
  `CST.MAC`; not traced.
- **`WV.TAB`'s extent.** 37 bytes are listed at `CWV.MAC:120`, and the index is
  `WV.XCO*4 + WV.YCO`, but the valid ranges of `WV.XCO`/`WV.YCO` and hence the
  world-map dimensions were not established.
- **`CT.CNT` initial value** (rows drawn per city) — set in `CR.INI`, not read.
- **The `0FF` padding.** Whether it is merely filler or is relied upon (e.g. as an
  end-of-data sentinel) is not established.
- The lost FORTRAN elevator-generation tool means elevator records can only be
  read, not regenerated — noted for completeness, not a blocker.
