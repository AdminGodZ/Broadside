#!/bin/sh
# Builds the browser game: the engine as WebAssembly, inlined into one page.
#   web/dist/broadside.html  open directly in a browser (no server needed)
#   web/dist/page.html       the same page without the document skeleton
set -e
cd "$(dirname "$0")/.."
cargo build --profile wasm -p broadside-wasm --target wasm32-unknown-unknown
mkdir -p web/dist
WASM=target/wasm32-unknown-unknown/wasm/broadside_wasm.wasm
python3 - "$WASM" <<'PY'
import base64, sys
wasm = base64.b64encode(open(sys.argv[1], "rb").read()).decode()
page = open("web/page.html").read().replace("__WASM__", wasm)
open("web/dist/page.html", "w").write(page)
open("web/dist/broadside.html", "w").write(
    '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
    '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n'
    '</head>\n<body>\n' + page + '\n</body>\n</html>\n')
PY
ls -l web/dist
