#!/bin/sh
# Build the verilated MiSTer Crystal Castles simulation.
#
# The RTL in Arcade-CrystalCastles_MiSTer is third-party reference material and
# is never modified. Verilator 5 is stricter than the 4.x era that core was
# written against, so the accommodations are all flags:
#
#   --no-timing   the RTL uses `#1` delays throughout, which are a simulation
#                 nicety rather than behaviour; verilator 5 refuses to guess.
#   -Wno-fatal    lint findings are reported but do not stop the build. They
#                 are about code that synthesises perfectly well for the FPGA
#                 the core actually targets.
#
# Everything is built under target/, so plain `cargo build` never needs
# verilator and the workspace stays dependency-free.

set -e

ROOT=$(cd "$(dirname "$0")/.." && pwd)
MISTER="$ROOT/Arcade-CrystalCastles_MiSTer"
SIM="$ROOT/crates/chill65-diff/sim"
OUT="$ROOT/target/mister-sim"

if [ ! -d "$MISTER/rtl" ]; then
    echo "build-mister-sim: $MISTER/rtl not found — clone the MiSTer core first" >&2
    exit 1
fi

# pll.v is Altera-specific and not reachable from the CCastles top anyway.
FILES=$(ls "$MISTER"/rtl/*.v | grep -v '/pll\.v$')
if [ -d "$MISTER/rtl/Pokey" ]; then
    FILES="$FILES $(ls "$MISTER"/rtl/Pokey/*.v)"
fi

mkdir -p "$OUT"
verilator --cc --exe --build \
    --top-module CCastles \
    --no-timing -Wno-fatal \
    -O2 \
    -Mdir "$OUT/obj" \
    -o mistersim \
    -y "$MISTER/rtl" -y "$MISTER/rtl/Pokey" \
    $FILES "$SIM/main.cpp"

cp "$OUT/obj/mistersim" "$OUT/mistersim"

# ColorMemory.v initialises from "cram.rom" by a relative $readmem, so the file
# has to sit in the simulation's working directory. Without it verilator warns
# and colour RAM starts as zeros.
if [ -f "$MISTER/rtl/cram.rom" ]; then
    cp "$MISTER/rtl/cram.rom" "$OUT/cram.rom"
fi

echo "build-mister-sim: $OUT/mistersim"
