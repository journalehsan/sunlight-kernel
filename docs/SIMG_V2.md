# SIMG v2 — Lossless Image Format for SunlightOS

**Status:** implemented (v2)  
**Endianness:** little-endian for all multi-byte integers  
**Goal:** backward-compatible, lossless, alpha-preserving, fast-decoding container

> **Legal / patent notice:** LZ4 and delta/filter preprocessing are widely used
> techniques, but the patent-free status of this combination as packaged here has
> **not** been legally reviewed for SunlightOS. Treat SIMG v2 as needing a
> formal legal review before redistribution claims of “patent free”. This is
> engineering documentation, not legal advice.

---

## Phase 1 findings (legacy pipeline)

These findings were recorded **before** finalizing the v2 header. Legacy
`.simg` assets must not be assumed to use a custom container.

| # | Question | Finding | Classification |
|---|----------|---------|----------------|
| 1 | What does an existing `.simg` file contain? | Uncompressed **TGA type-2** true-colour data (same layout as `.tga`). Sample pictures are 1312×816, 24 bpp BGR, top-down. | **compatibility constraint** |
| 2 | Own magic/header or renamed TGA? | **Renamed/raw TGA.** No SIMG magic. File(1) reports “Targa image data”. | **compatibility constraint** |
| 3 | How loader distinguishes TGA vs SIMG | **Extension only** for labels/MIME. Decode paths treat both as TGA type-2. | **compatibility constraint** |
| 4 | TGA support | Uncompressed type **2**, 24/32 bpp. RLE types 9–11 rejected. Color-map images unsupported in runtime UI decoder. | **reusable existing implementation** |
| 5 | Pixel layout | On disk: **BGR** (24) or **BGRA** (32). Decoder produces **ARGB8888** `u32` `(A<<24)\|(R<<16)\|(G<<8)\|B`, top-down. Stride = `width * bpp/8`, no row padding. Alpha is **straight** (not premultiplied). | **compatibility constraint** |
| 6 | Runtime decoders / build tools | Runtime: `sunlight-ui/image/tga.rs` (zero-alloc view), `sunlight-ui/image/simg.rs` (owned decode + scale). Host: `sun-img` / `sun-imgc`. Apps embed TGA via `include_bytes!` / `build.rs`. | **reusable existing implementation** |
| 7 | Asset packaging | Embedded in ramfs (`sunlight-fs`), `include_bytes!`, disk paths under `/usr/share/...`, thumbnails written as TGA-type `.simg`. | **compatibility constraint** |
| 8 | LZ4 in repo | Kernel ZRAM uses `lz4_flex` 0.11 (`default-features = false`) with `compress_into` / `decompress_into`. | **reusable existing implementation** (dependency only) |
| 9 | Suitable for user-space no_std? | Yes: `lz4_flex` block API is `no_std` when default features are off. **Do not** call into kernel ZRAM. | **architectural concern** (shared dep, not kernel coupling) |
| 10 | Allocation / max size | Owned decoder allocates final `Vec<u32>` once. No explicit max dimension; width×height can overflow on hostile headers. | **security/robustness defect** (mitigated in v2) |
| 11 | Call sites needing legacy | `decode_simg`, `TgaImage::parse`, Light Lens, File Manager preview/`draw_tga_bytes`, thumbd, rappid-rabbit, many `include_bytes!` icons. | **compatibility constraint** |

**Checksum inventory:** `crc32fast` appears in `sunlight-kv` (std/host). Kernel ZRAM uses FNV-1a, not CRC32. No small shared no_std CRC32 crate was wired for UI. SIMG v2 therefore ships a **tiny IEEE CRC-32** implementation for optional integrity over uncompressed pixels (not a substitute for authentication).

**Renderer note:** Premultiplied-alpha bilinear sampling and blend paths are unchanged. SIMG v2 decodes to the same straight-alpha ARGB representation; no implicit premultiply on load.

---

## Magic and version detection

| Bytes (LE) | Value |
|------------|-------|
| `0x53 0x49 0x4D 0x47` | ASCII `SIMG` |

- TGA type-2 files start with ID length / color-map type / image type. Image type
  `0x4D` (`'M'`) is **not** a valid TGA type, so a SIMG v2 file cannot be
  accepted by the legacy TGA parsers.
- Detection order for loaders that accept both:

  1. If magic == `SIMG` → **only** the SIMG v2 parser (never fall back to TGA).
  2. Else → legacy TGA type-2 parser (covers both `.tga` and historical `.simg`).

- A malformed SIMG v2 file (magic matched, validation failed) remains a **v2
  error**. It must not be reinterpreted as TGA.

Legacy “SIMG” files have **no** version field; they are TGA. SIMG v2 uses
`version = 2`. Version `0`/`1` are reserved and rejected.

---

## Byte layout (header)

All multi-byte fields are **little-endian**. Fields are written/read
**explicitly** (no `#[repr(C)]` platform struct dumps).

**Header size for this version: 36 bytes.**

| Offset | Size | Type | Field | Notes |
|--------|------|------|-------|-------|
| 0 | 4 | `[u8;4]` | `magic` | `b"SIMG"` |
| 4 | 2 | `u16` | `version` | Must be `2` |
| 6 | 2 | `u16` | `header_size` | Must be `36` for writers of this revision |
| 8 | 4 | `u32` | `flags` | See flags |
| 12 | 4 | `u32` | `width` | Pixels, nonzero, ≤ max |
| 16 | 4 | `u32` | `height` | Pixels, nonzero, ≤ max |
| 20 | 1 | `u8` | `pixel_format` | Enum |
| 21 | 1 | `u8` | `alpha_mode` | Enum |
| 22 | 1 | `u8` | `compression` | Enum |
| 23 | 1 | `u8` | `filter` | Enum |
| 24 | 4 | `u32` | `uncompressed_size` | Exact decoded pixel byte length |
| 28 | 4 | `u32` | `payload_size` | Exact stored payload length |
| 32 | 4 | `u32` | `crc32` | IEEE CRC-32 of **uncompressed** canonical pixels when flag set; else `0` |

Payload begins at byte `header_size` and is exactly `payload_size` bytes.
Trailing garbage after the payload may be ignored by decoders; writers must not
emit trailing data.

### Flags (`u32`)

| Bit | Name | Meaning |
|-----|------|---------|
| 0 | `FLAG_CRC32` (`1`) | `crc32` field is valid over uncompressed BGRA pixels |
| 1–31 | reserved | Must be **0** on write; unknown bits → reject |

### Pixel format (`u8`)

| Value | Name | Bytes/pixel | Channel order (memory) |
|-------|------|-------------|------------------------|
| 0 | reserved | — | — |
| **1** | `BGRA8` | 4 | B, G, R, A |
| 2–255 | reserved | — | reject |

`BGRA8` matches TGA 32 bpp channel order and maps cleanly to the UI’s ARGB
`u32` packing. Only `BGRA8` is fully implemented in v2.

### Alpha mode (`u8`)

| Value | Name | Meaning |
|-------|------|---------|
| 0 | reserved | — |
| **1** | `Straight` | Associated (straight) alpha; **no** premultiplication on disk |
| 2 | `Premultiplied` | reserved; reject in v2 |
| 3–255 | reserved | reject |

### Compression (`u8`)

| Value | Name | Payload |
|-------|------|---------|
| **0** | `None` / raw | Uncompressed pixel bytes (after filter, if any) |
| **1** | `Lz4` | Single LZ4 **block** (not LZ4 frame); no size prefix |
| 2–255 | reserved | reject |

LZ4 is the `lz4_flex` block format (`compress` / `decompress_into`). The
uncompressed length is taken **only** from the header, never from an
untrusted size-prepended stream alone.

### Filter (`u8`)

| Value | Name | Rule |
|-------|------|------|
| **0** | `None` | Identity |
| **1** | `Sub` | Per-row Sub (see below), then compress |
| 2–255 | reserved | reject |

### Valid compression × filter combinations

| Compression | Filter | Allowed |
|-------------|--------|---------|
| None | None | yes |
| Lz4 | None | yes |
| Lz4 | Sub | yes |
| None | Sub | **no** (Sub is only defined as a pre-LZ4 step in v2) |
| other | any | no |

---

## Pixel buffer layout

- **Row order:** top-down (row 0 = top of image).
- **Stride:** `width * 4` bytes; **no** row padding.
- **Canonical uncompressed size:** `width * height * 4` (checked for overflow).
- **Alpha:** straight; transparent pixels must preserve RGB bytes bit-exactly.
- Decoders for the UI convert BGRA bytes → ARGB `u32` without changing channel
  values (no premultiply).

---

## Sub filter (reversible)

Applied independently to each row of the **canonical BGRA8** buffer.

Let `bpp = 4` (bytes per pixel). For a row of `width` pixels (`row_len = width * 4`):

- Bytes `[0 .. bpp)` are stored unchanged.
- For each later byte index `i` in the row (`bpp .. row_len`):

  ```text
  filtered[i] = original[i].wrapping_sub(original[i - bpp])
  ```

- Reconstruction:

  ```text
  original[i] = filtered[i].wrapping_add(original[i - bpp])
  ```

- Filtering **resets every row**.
- Arithmetic is modulo 256 (`u8` wrapping).
- There is no row padding in BGRA8, so padding cannot become ambiguous.

---

## Compression rules

1. Canonical pixels = BGRA8 top-down straight alpha.
2. If filter is Sub, apply Sub to produce intermediate bytes of the same length.
3. If compression is None, payload = that buffer.
4. If compression is Lz4, payload = LZ4 block compress of that buffer.
5. Encoder candidates (deterministic order for ties — prefer earlier on equal size):

   1. Raw (None + None)
   2. LZ4 (Lz4 + None)
   3. Sub+LZ4 (Lz4 + Sub)

6. Choose the candidate with the **smallest complete file** (`header_size + payload_size`).
   If compressed size is not beneficial, store raw.
7. Same input + same encoder version ⇒ same output bytes.

---

## Decoder validation order

Reject at the first failure; do not allocate decoded buffers from untrusted
sizes until layout checks pass.

1. Need at least 4 bytes; magic must be `SIMG`.
2. Need at least 8 bytes; `version == 2`.
3. Need at least `header_size` bytes; `header_size == 36` for this revision.
4. `flags` only known bits (bit 0 optional CRC).
5. `width`, `height` nonzero and ≤ `MAX_DIMENSION` (8192).
6. `pixel_format == BGRA8`, `alpha_mode == Straight`.
7. Compression/filter combo valid.
8. `width * height` and `* 4` do not overflow; result ≤ `MAX_DECODED_BYTES` (64 MiB).
9. `uncompressed_size` equals computed size exactly.
10. `payload_size` fits in remaining file after header.
11. Allocate **one** output buffer of `uncompressed_size` (propagate OOM).
12. Raw: `payload_size == uncompressed_size`; copy payload → buffer.
13. LZ4: `decompress_into` into buffer; written length must equal `uncompressed_size`.
14. If filter Sub: reverse Sub **in place**.
15. If `FLAG_CRC32`: CRC-32 of buffer must match `crc32`.
16. Success.

**Limits:**

| Constant | Value |
|----------|-------|
| `MAX_DIMENSION` | 8192 |
| `MAX_DECODED_BYTES` | 64 × 1024 × 1024 |
| `HEADER_SIZE_V2` | 36 |

---

## Encoder (host / tooling)

- May allocate candidate buffers.
- Newly written files set `FLAG_CRC32` and fill `crc32`.
- Prefer decode simplicity over marginal size wins (decode is hotter than encode).

---

## Forward compatibility

- Unknown `version` → hard error.
- For `version == 2`, writers use `header_size == 36`. Readers require the same
  for this revision (no silent skip of unknown trailing header fields yet).
- Reserved enum values → hard error (no partial support).
- Future versions may grow the header or add formats; they must use a new
  `version` number.

---

## Backward compatibility strategy

| Asset class | Loader behavior |
|-------------|-----------------|
| Historical `.simg` (TGA bytes) | TGA path |
| `.tga` | TGA path |
| SIMG v2 raw / LZ4 / Sub+LZ4 | v2 path |
| Malformed v2 | v2 error only |

Do not mass-migrate assets until unit tests, corpus ratios, and a small
runtime proof pass. Keep a way to emit legacy TGA via existing tools.

---

## Non-goals (intentionally deferred)

Mipmaps, animation, GPU textures, progressive decode, layers, metadata
editing, encryption, general media containers, lossy quantization, palette
conversion, chroma subsampling, RLE as a required v2 method, coupling to
kernel ZRAM.

---

## Tooling

`sun-imgc` commands (see crate help):

- `inspect` — detect TGA / SIMG v2 and print fields
- `convert` — TGA/legacy → TGA (existing) or → SIMG v2
- `to-simg` — encode SIMG v2 with method selection + optional verify
- `from-simg` — decode SIMG v2 → TGA RGBA32
- `bench-corpus` — measure sizes/methods over a directory without overwriting sources
