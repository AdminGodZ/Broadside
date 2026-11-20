#!/bin/sh
# Builds the engine as WebAssembly and puts it where the play page imports it
# (src/components/game/broadside.wasm, not committed). Vite gives it a hashed
# name, so browsers cache it for good.
set -e
cd "$(dirname "$0")/../.."
cargo build --profile wasm -p broadside-wasm --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/wasm/broadside_wasm.wasm site/src/components/game/broadside.wasm
