# Compressed image fixtures

These tiny synthetic images were generated for this repository, with no external
artwork. JPEG and palette/RGBA PNG fixtures were encoded using Pillow 12.3.0.
The other PNGs use explicit PNG chunks with Python's `struct` and `zlib`.
Tests use committed bytes and do not require Python or Pillow.

- `rgba.png`, `palette.png`, `adam7.png`: 2x2, row-major RGBA values
  `(255,0,0,255)`, `(0,255,0,128)`, `(0,0,255,0)`, `(12,34,56,64)`.
  Palette PNG uses 2-bit indices; Adam7 encodes the three nonempty passes.
- `gray1.png`: 2x1 black/white, packed scanline `00 40` (filter then samples).
- `gray16.png`: 2x1 grayscale values `0x1234`, `0xabcd`.
- `rgb_trns.png`: 2x1 red/green with red declared transparent via `tRNS`.
- `baseline.jpg`, `progressive.jpg`: 8x8 RGB `(210,40,70)`, quality 95,
  default subsampling, with progressive encoding enabled only for the latter.
- `gray.jpg`: 8x8 grayscale 123, quality 95.
- `cmyk.jpg`: 8x8 CMYK `(0,255,255,0)`, quality 95 (red).
