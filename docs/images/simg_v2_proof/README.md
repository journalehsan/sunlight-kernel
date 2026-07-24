# SIMG v2 representative proof assets

These files were converted with `sun-imgc to-simg --verify` for SIMG v2
integration testing. **They are not a mass migration** of the asset tree.

| File | Source | Method |
|------|--------|--------|
| `01_solar_blossom.simg` | `docs/images/Samples/01_solar_blossom.simg` | sub+lz4 |
| `05_sleepy_koala.simg` | `docs/images/Samples/05_sleepy_koala.simg` | sub+lz4 |
| `system-run.simg` | `docs/icons/SunlightOS/apps/48/system-run.tga` | lz4 |
| `applications-system.simg` | `docs/icons/SunlightOS/apps/48/applications-system.tga` | lz4 |
| `wallpaper.simg` | `docs/images/wallpaper.tga` | sub+lz4 |

Each file was verified byte-for-byte against the decoded legacy source at encode
time (`--verify`).

Runtime loaders (`decode_simg`, Light Lens, File Manager preview, thumbd) accept
these beside legacy TGA/SIMG. Full desktop screenshot verification requires a
bootable graphical session; host tests prove lossless decode only.
