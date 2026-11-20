#!/bin/sh
# Builds the browser game (web/build.sh) and puts it in public/play/, where
# Astro copies it unchanged: the site serves it at /play.
set -e
cd "$(dirname "$0")/.."
../web/build.sh
mkdir -p public/play
cp ../web/dist/broadside.html public/play/index.html
