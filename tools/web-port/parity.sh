#!/usr/bin/env sh
# Native vs wasm32 bit parity for the engine crates (docs/web-port/determinism.md).
# Needs the wasm32-unknown-unknown target, wasm-bindgen-cli 0.2.127 and Node 22.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
out="$root/target/web-parity"
cd "$root"
mkdir -p "$out"

cargo build --release --locked -p skate-wasm-parity
target/release/skate-wasm-parity --values "$out/values" > "$out/native.txt"
SKATE_AUDIO_FMA=0 target/release/skate-wasm-parity > "$out/native-plain.txt"

cargo rustc --release --locked -p skate-wasm-parity --lib --target wasm32-unknown-unknown --crate-type cdylib
wasm-bindgen --target nodejs --out-dir "$out/node" target/wasm32-unknown-unknown/release/skate_wasm_parity.wasm

node tools/web-port/parity.mjs "$out"
