#!/bin/sh
# Vercel's build image has Node but no Rust: install rustup on first use (the
# toolchain, its components and the wasm target come from rust-toolchain.toml),
# then build the site with Astro. The output is site/dist.
set -e
if ! command -v cargo >/dev/null 2>&1; then
  [ -x "$HOME/.cargo/bin/cargo" ] || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain none
  PATH="$HOME/.cargo/bin:$PATH"
  export PATH
fi
cd "$(dirname "$0")/.."
rustup toolchain install
cd site
npm ci --no-audit --no-fund
npm run build
