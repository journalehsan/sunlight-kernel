# Btop for SunlightOS

The boot image publishes this format-1 bundle at `/Applications/Btop.sunapp`.
Type `btop` at a SunShell prompt to run the monitor in that session, or select
Btop in Start to open it in a new Sunlight Terminal window. When Btop exits,
the new terminal remains available at its shell prompt.

The bundle requests no additional grants. Btop runs with the current user's
existing Helios filesystem, process and terminal access. The bundle uses the
built-in themes; it needs no dynamic libraries or Linux distribution.

The payload is Btop v1.4.5, upstream revision
`a1099d075fa914452366b28c67ff171e21a1f02e` from `aristocratos/btop`, with SHA-256
`6632acc21bb187f692df3fe09c13a36ce97fe5775698bcc937342603801cce29`.
The build uses Alpine musl 1.2.5-r24 and GCC 15.2.0-r2. Apply
`Resources/ifaddrs-error.patch` from the Btop source checkout before building.
It prevents the network collector from freeing an unowned pointer when
interface discovery fails, including when no network backend is available.

```sh
make PLATFORM=linux STATIC=true GPU_SUPPORT=false INTEL_GPU_SUPPORT=false \
    STRIP=true ADDFLAGS="-fno-pie -no-pie"
bash tools/stamp_helios_elf.sh Btop.sunapp/Program/btop
```

Run `python3 tools/test_btop_ifaddrs.py <patched-btop-checkout>/src/btop_shared.hpp`
from the SunlightOS checkout to test failed, empty, and successful discovery.

This is a boot image application. The current installation model has no
independent install/uninstall transaction for image applications.
