#!/usr/bin/env python3
"""Check DN file-mask cell backgrounds in the two-window 1280x800 replay."""
from pathlib import Path
import sys
from PIL import Image

root = Path(sys.argv[1])
plain = (12, 20, 37)
highlight = (111, 162, 230)
for side in ('shift', 'shift_r', 'both'):
    expected = {'before-loss': 'c', 'regained': 'c', 'ordinary-held': '',
                'ordinary-released': '', 'fresh-make': 'b', 'fresh-break': ''}
    expected.update({'release-shift': '', 'release-shift_r': ''} if side == 'both'
                    else {f'release-{side}': ''})
    for frame, wanted in expected.items():
        name = f'{side}-{frame}'
        with Image.open(root / f'{name}.png') as image:
            image = image.convert('RGB')
            colors = [image.getpixel((627 + 9 * i, 369)) for i in range(6)]
        assert set(colors) <= {plain, highlight}, (name, colors)
        selected = ''.join(c for c, color in zip('abcdef', colors) if color == highlight)
        print(f'{name}: selected={selected!r} expected={wanted!r}', flush=True)
        assert selected == wanted, name
