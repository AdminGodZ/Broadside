#!/bin/sh
# Builds the website into site/dist:
#   index.html        the landing page (site/index.html, version filled in)
#   play/index.html   the browser game (web/build.sh)
#   *.svg             logo and the duel chart from assets/
set -e
cd "$(dirname "$0")/.."
web/build.sh
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$VERSION" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
rm -rf site/dist
mkdir -p site/dist/play
sed "s/__VERSION__/$VERSION/g" site/index.html > site/dist/index.html
cp web/dist/broadside.html site/dist/play/index.html
cp assets/logo-dark.svg assets/logo-light.svg assets/duel-parity.svg site/dist/
echo "site/dist built for v$VERSION"
