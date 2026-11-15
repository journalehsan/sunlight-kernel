#!/usr/bin/env bash
set -euo pipefail
audit_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
build_root="${1:?Pass a new directory under /tmp or target}"
mkdir "$build_root"
cd "$build_root"
curl -L --fail https://download.qemu.org/qemu-11.1.2.tar.xz -o qemu.tar.xz
printf '%s  qemu.tar.xz\n' '731b5681e4bb18be313231579b8efd0296c5b015fa36dc533874b639ba838016' | sha256sum -c -
mkdir source build
# Match the development extraction's rejection of absolute symlinks.
python3 - <<'PY'
import tarfile
with tarfile.open('qemu.tar.xz') as archive:
    for entry in archive.getmembers():
        entry.name = entry.name.split('/', 1)[-1]
        if entry.name and not (entry.issym() and entry.linkname.startswith('/')):
            archive.extract(entry, 'source', filter='data')
PY
patch -d source -p1 < "$audit_dir/qemu-async-access.patch"
cd build
../source/configure --target-list=x86_64-softmmu --disable-docs --disable-werror --disable-download --disable-gtk --disable-sdl --disable-opengl --disable-virglrenderer --disable-vnc --disable-tools --disable-user --disable-plugins
ninja -j "${SFS5B4_QEMU_JOBS:-1}" qemu-system-x86_64
