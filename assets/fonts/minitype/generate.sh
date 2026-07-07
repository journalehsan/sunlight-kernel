#!/usr/bin/env bash
# Generate SunlightOS MiniType font files from source TTFs.
#
# The sun-font build.rs does this automatically at cargo build time.
# Run this script only if you need standalone .mtf files (e.g. for testing
# the dynamic font loader or adding fonts to the OS image manually).
#
# Primary method (no extra tools): trigger sun-font build.rs (fontdue) and
# copy the produced .mtf files here.
#
# Optional: cargo install minitype-cli   then the CLI lines below also work
# and support --range for full Material Icons PUA etc.

set -e

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
OUT_DIR="$REPO_ROOT/assets/fonts/minitype"

mkdir -p "$OUT_DIR"

# Preferred / reliable path: the in-tree generator in sun-font/build.rs
# (uses fontdue). It supports both the UI fonts and Material Icons conversion.
echo "Building sun-font to (re)generate all .mtf (including Material Icons)..."
cargo build -p sun-font

# Locate the most recent sun-font OUT_DIR and copy the fonts.
OUT_BUILD=$(find "$REPO_ROOT/target" -path '*sun-font*/out/sunlight_ui_13.mtf' | head -1 | xargs dirname || true)
if [ -n "$OUT_BUILD" ] && [ -d "$OUT_BUILD" ]; then
  cp "$OUT_BUILD"/sunlight_*.mtf "$OUT_BUILD"/material_icons_*.mtf "$OUT_DIR/" 2>/dev/null || true
  echo "Copied generated .mtf from $OUT_BUILD"
else
  echo "Warning: could not locate sun-font OUT_DIR"
fi

# --- Optional: minitype-cli (external tool) ---
# Produces the MFNT container format used by the upstream minitype crate.
# Currently the published 0.0.1 can be unstable with some TTFs / envs (LoadFailed inside its rasterizer).
# The script will only use it for Inter if a quick smoke test passes.
if command -v minitype >/dev/null 2>&1; then
  INTER_DIR="$REPO_ROOT/docs/fonts/Inter/static"
  MATERIAL_DIR="$REPO_ROOT/assets/fonts/Material-Icons"

  # Quick smoke test on a known-good glyph range.
  # Note: the published minitype-cli 0.0.1 can panic inside its WebRender rasterizer
  # for some fonts (e.g. Inter). We swallow errors and only use successful runs.
  if minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 13 --range 'A:Z' -o /tmp/minitype_smoke.mtf >/dev/null 2>&1; then
    minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 11 -o "$OUT_DIR/sunlight_ui_11.mtf" 2>/dev/null || true
    minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 13 -o "$OUT_DIR/sunlight_ui_13.mtf" 2>/dev/null || true
    minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 16 -o "$OUT_DIR/sunlight_ui_16.mtf" 2>/dev/null || true
    minitype --ttf "$INTER_DIR/Inter_18pt-Medium.ttf" --size 13 -o "$OUT_DIR/sunlight_ui_medium_13.mtf" 2>/dev/null || true
    minitype --ttf "$INTER_DIR/Inter_18pt-Regular.ttf" --size 13 -o "$OUT_DIR/sunlight_mono_13.mtf" 2>/dev/null || true
  fi
  rm -f /tmp/minitype_smoke.mtf 2>/dev/null || true

  # === Material Icons (the key part requested) ===
  # The icon font lives in PUA (U+E000+). Use the largest populated ranges.
  # minitype-cli succeeds for these when given exact populated ranges.
  echo "Converting Material Icons via minitype (if available)..."
  if minitype --ttf "$MATERIAL_DIR/MaterialIcons-Regular.ttf" --size 16 \
       --range $'\uea09:\uea69' --range $'\ue226:\ue26c' \
       -o /tmp/mi_smoke.mtf >/dev/null 2>&1; then
    minitype --ttf "$MATERIAL_DIR/MaterialIcons-Regular.ttf" --size 16 \
      --range $'\uea09:\uea69' --range $'\ue226:\ue26c' \
      --range $'\ue39d:\ue3e0' --range $'\ue875:\ue8b6' \
      -o "$OUT_DIR/material_icons_16.mtf" 2>/dev/null || true

    minitype --ttf "$MATERIAL_DIR/MaterialIcons-Regular.ttf" --size 24 \
      --range $'\uea09:\uea69' --range $'\ue226:\ue26c' \
      --range $'\ue875:\ue8b6' \
      -o "$OUT_DIR/material_icons_24.mtf" 2>/dev/null || true

    echo "Material Icons converted via minitype-cli (MFNT format)"
  else
    echo "minitype-cli could not process Material Icons (or not present) — keeping in-tree version"
  fi
  rm -f /tmp/mi_smoke.mtf 2>/dev/null || true
fi

echo "Generated MiniType font files in $OUT_DIR"
ls -lh "$OUT_DIR"/*.mtf 2>/dev/null || ls -lh "$OUT_DIR"/
