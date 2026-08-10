#!/usr/bin/env bash
#
# Assemble the browser build in target/web.
#
#   bash tools/stage-web.sh
#   python3 -m http.server --directory target/web 8000
#   # then open http://localhost:8000/
#
# A server is required: `fetch` of a file:// URL is blocked, and
# `instantiateStreaming` wants a real application/wasm content type.
#
# The staged directory mixes two kinds of file, which is exactly why it lives
# under gitignored target/ (plan §9):
#
#   index.html, app.js   ours, committed
#   chill65.wasm         built from the game source — game-derived
#   prog.bin, data.bin   the ROM images — game-derived
#   mob.bin              the motion-object picture ROMs — game-derived
#
# Nothing here may be copied outside target/ or committed. Build the inputs
# first, both with the corpus set:
#
#   cargo build -p chill65-wasm --target wasm32-unknown-unknown --release
#   cargo run -p chill65-native --bin ccnative -- hashes --trace traces/idle-attract.trace
#
# The second writes the boot artefacts as a side effect.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/target/web"

wasm="$root/target/wasm32-unknown-unknown/release/chill65_wasm.wasm"
prog="$root/target/boot-artefacts/prog.bin"
data="$root/target/boot-artefacts/data.bin"
mob="$root/target/boot-artefacts/mob.bin"

need() {
    if [ ! -f "$1" ]; then
        echo "stage-web: missing $1" >&2
        echo "           $2" >&2
        exit 1
    fi
}

need "$root/web/index.html" "this repository is incomplete"
need "$root/web/app.js" "this repository is incomplete"
need "$wasm" "cargo build -p chill65-wasm --target wasm32-unknown-unknown --release"
need "$prog" "cargo run -p chill65-native --bin ccnative -- hashes --trace traces/idle-attract.trace"
need "$data" "cargo run -p chill65-native --bin ccnative -- hashes --trace traces/idle-attract.trace"
need "$mob" "cargo run -p chill65-native --bin ccnative -- hashes --trace traces/idle-attract.trace"

mkdir -p "$out"
cp "$root/web/index.html" "$out/index.html"
cp "$root/web/app.js" "$out/app.js"
cp "$wasm" "$out/chill65.wasm"
cp "$prog" "$out/prog.bin"
cp "$data" "$out/data.bin"
cp "$mob" "$out/mob.bin"

echo "staged into $out"
ls -l "$out"
echo
echo "serve it with:  python3 -m http.server --directory target/web 8000"
echo "then open:      http://localhost:8000/"
