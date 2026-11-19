# Chronos virtual BIOS fonts

Source: https://github.com/viznut/unscii/tree/257b4dfeb50651510fe8b9cf4f151a148a4490e4
Author: Ville-Matias Heikkila (Viznut) and Unscii contributors.
License: Public Domain or CC0, per retained UPSTREAM-README.md. Redistribution
and modification permitted, without mandatory attribution or share-alike.
Only fontfiles/unscii-8.hex and unscii-16.hex were used, never GPL unscii-16-full.

Conversion maps Unicode to CP437 (graphical controls, house at 7F, NUL blank).
Four symbols missing upstream are original SunlightOS CC0 drawings: sun (0F),
house (7F), reversed not sign (A9), bullet operator (F9). Their 8-row bitmaps:
24995a3c3c5a9924, 00183c6666427e00, 00007e0606060000, 0000001818000000.
These four are vertically doubled for 16 rows; other glyphs use upstream native
height. Binary layout: 256 glyphs, 8/16 bytes per glyph, one byte per scanline,
MSB left, no header. These are complete CP437-compatible virtual adapter fonts,
not pixel-identical IBM ROM dumps or placeholder fonts.

8x14 and alternate 9-dot replacement tables are unsupported. Add lawful native
height assets and conformance tests before exposing those selectors.
