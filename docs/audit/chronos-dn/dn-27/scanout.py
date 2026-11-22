"""Read stable background pixels of the six mask character cells (1280x800 replay)."""
from pathlib import Path
import sys

from PIL import Image

TEXT = "abcdef"
PLAIN = (12, 20, 37)
HIGHLIGHT = (111, 162, 230)
FRAMES = {
    "plain-right": "",
    "plain-left": "",
}
for side in ("shift", "shift_r"):
    for i, (arrow, selection) in enumerate([
        ("left", "c"), ("left", "bc"), ("right", "c"), ("right", ""),
        ("right", "d"), ("right", "de"), ("left", "d"),
    ]):
        FRAMES[f"{side}-{i}-{arrow}"] = selection
    FRAMES[f"{side}-released-before-arrow"] = "de"
    FRAMES[f"{side}-plain-after-release"] = ""

mode, directory = sys.argv[1:]
assert mode in {"baseline", "corrected"}
def glyphs(frame, colors):
    # Exclude the caret underline and compare glyph occupancy regardless of
    # the foreground and background palette used for selection.
    return tuple(
        tuple(frame.getpixel((598 + i * 9 + x, y)) != color
              for y in range(342, 353) for x in range(9))
        for i, color in enumerate(colors)
    )


original = None
for name, wanted in FRAMES.items():
    with Image.open(Path(directory) / f"{name}.png") as frame:
        assert frame.size == (1280, 800)
        frame = frame.convert("RGB")
        colors = [frame.getpixel((599 + i * 9, 341)) for i in range(len(TEXT))]
        assert set(colors) <= {PLAIN, HIGHLIGHT}, f"unexpected palette in {name}"
        shape = glyphs(frame, colors)
        if original is None:
            original = shape
            assert all(any(cell) for cell in original)
        assert shape == original, f"mask glyphs changed in {name}"
        selected = "".join(char for char, color in zip(TEXT, colors) if color == HIGHLIGHT)
        expected = wanted if mode == "corrected" else ""
        print(f"{name:<34} selected={selected!r} expected={expected!r} background={colors}")
        assert selected == expected, name
