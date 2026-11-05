#!/usr/bin/env bash
# Render the SunlightOS cursor SVGs (docs/images/cursors/*.svg) to 32x32
# TGA type-2 32bpp top-down assets in assets/cursors/, ready for
# include_bytes! + TgaImage::parse in sunlight-display.
#
# Requires: rsvg-convert, python3 + Pillow.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SVG_DIR="$PROJECT_ROOT/docs/images/cursors"
OUT_DIR="$PROJECT_ROOT/assets/cursors"
SIZE=32

mkdir -p "$OUT_DIR"

for svg in "$SVG_DIR"/*.svg; do
    name="$(basename "$svg" .svg)"
    png="$(mktemp --suffix=.png)"
    rsvg-convert -w "$SIZE" -h "$SIZE" "$svg" -o "$png"
    python3 - "$png" "$OUT_DIR/$name.tga" <<'EOF'
import struct, sys
from PIL import Image

png_path, tga_path = sys.argv[1], sys.argv[2]
img = Image.open(png_path).convert("RGBA")
w, h = img.size

# TGA type 2 (uncompressed truecolor), 32 bpp, top-down (descriptor 0x28:
# bit5 = upper-left origin, low nibble = 8 alpha bits). Matches the
# sunlight-ui TgaImage parser.
header = struct.pack(
    "<BBBHHBHHHHBB",
    0,      # id length
    0,      # no color map
    2,      # uncompressed truecolor
    0, 0, 0,  # color map spec
    0, 0,   # x/y origin
    w, h,
    32,     # bpp
    0x28,   # descriptor: top-down + 8 attribute bits
)
rows = []
px = img.load()
for y in range(h):
    row = bytearray()
    for x in range(w):
        r, g, b, a = px[x, y]
        row += bytes((b, g, r, a))
    rows.append(bytes(row))
with open(tga_path, "wb") as f:
    f.write(header + b"".join(rows))
print(f"  {tga_path} ({w}x{h})")
EOF
    rm -f "$png"
done

echo "[gen_cursors] done"
