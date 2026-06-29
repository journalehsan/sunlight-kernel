#!/usr/bin/env bash
# Generate SunlightOS MiniType font files from source TTFs.
#
# The sun-font build.rs does this automatically at cargo build time.
# Run this script only if you need standalone .mtf files (e.g. for testing
# the dynamic font loader or adding fonts to the OS image manually).
#
# Prerequisites:
#   cargo install minitype-cli
#
# Alternatively, use fontdue in a custom tool or let the build.rs handle it.

set -e

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
INTER_DIR="$REPO_ROOT/docs/fonts/Inter/static"
OUT_DIR="$REPO_ROOT/assets/fonts/minitype"

mkdir -p "$OUT_DIR"

# Inter Regular at 11, 13, 16 px
minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 11 -o "$OUT_DIR/sunlight_ui_11.mtf"
minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 13 -o "$OUT_DIR/sunlight_ui_13.mtf"
minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 16 -o "$OUT_DIR/sunlight_ui_16.mtf"

# Inter Medium at 13 px
minitype --ttf "$INTER_DIR/Inter_18pt-Medium.ttf" --size 13 -o "$OUT_DIR/sunlight_ui_medium_13.mtf"

# Mono: Inter Regular at 13 px until JetBrains Mono is available
minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 13 -o "$OUT_DIR/sunlight_mono_13.mtf"

echo "Generated MiniType font files in $OUT_DIR"
ls -lh "$OUT_DIR"/*.mtf
