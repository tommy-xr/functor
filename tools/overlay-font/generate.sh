#!/usr/bin/env bash
# Regenerates the headset overlay's MSDF font
# (runtime/functor-runtime-common/src/overlay_font/atlas.png + metrics.rs)
# from the pinned Inter release. Offline tooling: nothing here runs at build
# or load time.
#
#   tools/overlay-font/generate.sh
set -euo pipefail

VERSION=4.1
URL="https://github.com/rsms/inter/releases/download/v${VERSION}/Inter-${VERSION}.zip"
SHA256=9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e

here="$(cd "$(dirname "$0")" && pwd)"
out="$here/../../runtime/functor-runtime-common/src/overlay_font"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

curl -fsSL -A "overlay-font-generator/1.0" -o "$work/inter.zip" "$URL"
echo "$SHA256  $work/inter.zip" | shasum -a 256 -c -
unzip -q -o "$work/inter.zip" LICENSE.txt extras/ttf/Inter-Regular.ttf extras/ttf/Inter-SemiBold.ttf -d "$work"
cp "$work/LICENSE.txt" "$out/OFL-Inter.txt"
cargo run --release --manifest-path "$here/Cargo.toml" -- \
  "$work/extras/ttf/Inter-Regular.ttf" "$work/extras/ttf/Inter-SemiBold.ttf" "$out"
