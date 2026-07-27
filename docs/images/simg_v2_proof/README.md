# SIMG v2 representative proof assets

These files were converted with `sun-imgc to-simg --verify` for SIMG v2
integration testing.

| File | Source | Method |
|------|--------|--------|
| `01_solar_blossom.simg` | `docs/images/Samples/01_solar_blossom.simg` | sub+lz4 |
| `05_sleepy_koala.simg` | `docs/images/Samples/05_sleepy_koala.simg` | sub+lz4 |
| `system-run.simg` | `docs/icons/SunlightOS/apps/48/system-run.tga` | lz4 |
| `applications-system.simg` | `docs/icons/SunlightOS/apps/48/applications-system.tga` | lz4 |
| `wallpaper.simg` | `docs/images/wallpaper.tga` | sub+lz4 |

Desktop wallpapers and the login background live next to their sources as
`docs/images/wallpaper{,1-4}.simg` and
`docs/images/sunlight-login-background.simg` (also staged into ramfs / embedded
for boot). Each was verified byte-for-byte against the decoded legacy TGA at
encode time (`--verify`).

Runtime loaders (`decode_simg`, Vortex wallpaper, lock presenter, tty login)
accept SIMG v2 beside legacy TGA.
