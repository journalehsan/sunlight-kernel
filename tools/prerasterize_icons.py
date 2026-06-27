#!/usr/bin/env python3
"""
SunlightOS Icon Pre-rasterizer
Converts SVG icons to 32-bit TGA (with alpha channel) for the SunlightOS kernel/shell.

Usage examples:
  # Convert a folder (keeps SVGs, writes .tga next to them)
  python3 tools/prerasterize_icons.py docs/icons/SunlightOS/some-category

  # Bulk convert the whole icon set safely to a separate tree (recommended)
  python3 tools/prerasterize_icons.py docs/icons/SunlightOS --out-dir target/icons-256 --quiet

  # Original destructive behavior (removes SVGs after conversion)
  python3 tools/prerasterize_icons.py . --delete-svg

Dependencies:
  - System (Arch): sudo pacman -S python-cairosvg python-pillow
  - Or venv: python -m venv v && v/bin/pip install cairosvg pillow

The script preserves directory structure when using --out-dir.
"""

import argparse
import os
import sys
from pathlib import Path
import cairosvg
from PIL import Image
import io

# Standard OS icon resolution. Adjust if your shell uses a different grid (e.g., 128x128 or 512x512)
DEFAULT_ICON_SIZE = (256, 256)


def find_svg_files(root: Path, recursive: bool) -> list[Path]:
    if recursive:
        candidates = root.rglob('*.svg')
    else:
        candidates = root.glob('*.svg')
    # Only real files (skip broken symlinks and odd entries that may exist in icon themes)
    return sorted(p for p in candidates if p.is_file())


def process_icons(target_dir: Path, out_dir: Path | None, icon_size: tuple[int, int], recursive: bool, delete_svg: bool, quiet: bool = False):
    svg_files = find_svg_files(target_dir, recursive)

    if not svg_files:
        print("🔍 No .svg files found.")
        return

    mode_str = "recursive" if recursive else "non-recursive"
    if out_dir:
        print(f"🚀 Found {len(svg_files)} SVG icons under {target_dir} ({mode_str}).")
        print(f"   Output directory: {out_dir}")
    else:
        print(f"🚀 Found {len(svg_files)} SVG icons under {target_dir} ({mode_str}).")
    print(f"   Target size: {icon_size[0]}x{icon_size[1]}")

    if out_dir:
        print("   Mode: Writing TGAs to output directory (sources unchanged).\n")
    elif delete_svg:
        print("   Mode: SVG will be REMOVED after successful TGA creation.\n")
    else:
        print("   Mode: SVGs will be KEPT (TGA files created alongside).\n")

    success_count = 0
    fail_count = 0

    for svg_path in svg_files:
        try:
            if out_dir:
                rel = svg_path.relative_to(target_dir)
                tga_path = (out_dir / rel).with_suffix('.tga')
                tga_path.parent.mkdir(parents=True, exist_ok=True)
            else:
                tga_path = svg_path.with_suffix('.tga')

            # 1. Convert SVG to PNG in memory (CairoSVG doesn't do TGA directly)
            png_data = cairosvg.svg2png(
                url=str(svg_path),
                output_width=icon_size[0],
                output_height=icon_size[1]
            )

            # 2. Open the in-memory PNG with Pillow
            with Image.open(io.BytesIO(png_data)) as img:
                # Ensure it's in RGBA mode for TGA transparency
                if img.mode != 'RGBA':
                    img = img.convert('RGBA')

                # 3. Save as TGA (Pillow handles the 18-byte TGA header automatically)
                img.save(tga_path, format='TGA')

            # 4. Verify TGA was created
            if tga_path.exists() and tga_path.stat().st_size > 0:
                if not quiet:
                    if out_dir:
                        print(f"✅ {rel} -> {tga_path}")
                    elif delete_svg:
                        os.remove(svg_path)
                        print(f"✅ {svg_path.relative_to(target_dir)} -> {tga_path.name} (SVG removed)")
                    else:
                        print(f"✅ {svg_path.relative_to(target_dir)} -> {tga_path.name}")
                success_count += 1
            else:
                rel_str = str(rel) if out_dir else str(svg_path.relative_to(target_dir))
                print(f"❌ {rel_str}: TGA generation failed (empty file).")
                fail_count += 1

        except Exception as e:
            rel_str = str(svg_path.relative_to(target_dir))
            print(f"❌ {rel_str}: Error processing -> {e}")
            fail_count += 1

    print(f"\n🎉 Conversion complete! {success_count} TGA icons generated.")
    if fail_count > 0:
        print(f"⚠️  {fail_count} icons failed to convert.")
    if not out_dir and not delete_svg and success_count > 0:
        print("   Original SVG sources were preserved.")


def main():
    parser = argparse.ArgumentParser(
        description="SunlightOS Icon Pre-rasterizer: Convert SVG icons to 256x256 TGA (RGBA)."
    )
    parser.add_argument(
        "directory",
        nargs="?",
        default=".",
        help="Directory containing SVG files (default: current directory)"
    )
    parser.add_argument(
        "-r", "--recursive",
        action="store_true",
        default=True,
        help="Recurse into subdirectories (default: on)"
    )
    parser.add_argument(
        "--no-recursive",
        dest="recursive",
        action="store_false",
        help="Disable recursion (only process *.svg directly in the target directory)"
    )
    parser.add_argument(
        "--size",
        default="256x256",
        help="Output icon size, e.g. 256x256 or 128x128 (default: 256x256)"
    )
    parser.add_argument(
        "--out-dir",
        default=None,
        help="Write TGA files under this directory, mirroring the source tree (recommended for bulk conversion)"
    )
    parser.add_argument(
        "-q", "--quiet",
        action="store_true",
        help="Only print errors and the final summary (recommended for large batches)"
    )
    parser.add_argument(
        "--delete-svg",
        action="store_true",
        default=False,
        help="Remove original SVG after successful TGA creation (destructive, only valid without --out-dir, off by default)"
    )

    args = parser.parse_args()

    # Parse size
    try:
        w, h = args.size.lower().split('x')
        icon_size = (int(w), int(h))
    except Exception:
        print(f"Invalid --size '{args.size}'. Use e.g. 256x256")
        sys.exit(1)

    target = Path(args.directory).resolve()
    if not target.is_dir():
        print(f"Error: {target} is not a directory")
        sys.exit(1)

    out_dir = Path(args.out_dir).resolve() if args.out_dir else None
    if out_dir:
        out_dir.mkdir(parents=True, exist_ok=True)

    if args.delete_svg and out_dir:
        print("Error: --delete-svg cannot be used together with --out-dir")
        sys.exit(1)

    process_icons(target, out_dir, icon_size, args.recursive, args.delete_svg, quiet=args.quiet)


if __name__ == "__main__":
    main()
