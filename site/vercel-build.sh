#!/bin/sh
# Vercel's build image has no Rust: install rustup on first use (the toolchain,
# its components and the wasm target come from rust-toolchain.toml), then build.
set -e
if ! command -v cargo >/dev/null 2>&1; then
  [ -x "$HOME/.cargo/bin/cargo" ] || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain none
  PATH="$HOME/.cargo/bin:$PATH"
  export PATH
fi
cd "$(dirname "$0")/.."
rustup toolchain install
exec site/build.sh
