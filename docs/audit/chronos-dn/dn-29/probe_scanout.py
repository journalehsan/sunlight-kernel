"""Decode the probe's hexadecimal text directly from captured scanout pixels."""

from pathlib import Path
import sys

from PIL import Image


def masks(path, top, count):
    with Image.open(path) as frame:
        gray = frame.convert("L")
        return [
            tuple(gray.getpixel((309 + 9 * col + x, top + y)) > 110
                  for y in range(16) for x in range(9))
            for col in range(count)
        ]


def verify(root):
    digits = "0123456789ABCDEF"
    glyphs = dict(zip(masks(root / "initial.png", 542, len(digits)), digits))
    assert len(glyphs) == len(digits)
    glyphs[(False,) * 144] = " "
    expected = {"initial": 0}
    for side, first, second in (("shift", 2, 1), ("shift_r", 1, 2)):
        for suffix, bits in [
            ("make", first), ("both", 3), ("first-break", second), ("second-break", 0),
            ("before-focus-loss", 3), ("focus-lost", 0), ("focus-regained", 0),
            ("delayed-first-break", 0), ("delayed-held-snapshot", 0),
            ("delayed-second-break", 0), ("fresh-make", second), ("fresh-break", 0),
        ]:
            expected[f"{side}-{suffix}"] = bits
    for name, bits in expected.items():
        row = "".join(glyphs[mask] for mask in masks(root / f"{name}.png", 574, 24))
        wanted = f"00D{bits} 020{bits} 0247 000{bits} 0247"
        assert row == wanted, (name, row, wanted)
        print(f"{name:32} {row}")
    print(f"{len(expected)} native scanout samples matched")


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
