# SunlightOS Color Image Conversion Foundation

This is the minimal user-space foundation for future color image assets such as wallpapers. It is intentionally separate from the monochrome icon pipeline.

## Scope

- Library crate: `sun-img`
- CLI tool: `sun-imgc`
- Canonical output: uncompressed 32-bit TGA with top-left origin
- Internal pixels: RGBA8

This task does not add wallpaper settings, Bing download, desktop menu integration, or a large image framework.

## Current support

`sun-img` currently implements:

- format detection for TGA, BMP, PNG, and JPEG
- decoding for uncompressed true-color TGA only
- encoding for canonical 32-bit TGA only
- convert-to-canonical-TGA for TGA input only

PNG, JPEG, and BMP decode paths intentionally return clean unsupported errors for now.

## CLI

Inspect a file:

`sun-imgc inspect assets/default.tga`

Convert to canonical TGA32:

`sun-imgc convert assets/default.tga generated/default.tga`

`convert` writes atomically by using a temporary sibling file and renaming it into place.

## Build.rs usage later

Future app or asset build steps can call the tool from `build.rs`:

```rust
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/wallpapers/default.tga");

    let status = Command::new("sun-imgc")
        .args([
            "convert",
            "assets/wallpapers/default.tga",
            "generated/wallpapers/default.tga",
        ])
        .status()
        .expect("failed to run sun-imgc");

    assert!(status.success());
}
```

Preferred convention is still to generate assets at build time and keep runtime code free of source image decoding when possible.
