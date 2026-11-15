#!/usr/bin/env bash
set -euo pipefail
# Isolated paths only; never installs or changes the system emulator.
audit_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
build_root="${1:?Pass a new, empty build directory under /tmp or target}"
mkdir "$build_root"
cd "$build_root"
curl -L --fail https://download.qemu.org/qemu-11.1.2.tar.xz -o qemu.tar.xz
printf '%s  qemu.tar.xz\n' '731b5681e4bb18be313231579b8efd0296c5b015fa36dc533874b639ba838016' | sha256sum -c -
mkdir source build
tar -xf qemu.tar.xz -C source --strip-components=1
patch -d source -p1 < "$audit_dir/qemu-active-io.patch"
cd build
../source/configure --target-list=x86_64-softmmu --disable-docs --disable-werror --disable-download --disable-gtk --disable-sdl --disable-opengl --disable-virglrenderer --disable-vnc --disable-tools --disable-user --disable-plugins
ninja -j 6 qemu-system-x86_64
