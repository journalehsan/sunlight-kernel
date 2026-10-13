#!/usr/bin/env bash
set -euo pipefail

# Supply the extracted upstream Ghostscript 10.08.0 source. Downloads and build
# artifacts stay out of the repository. Initialization files/fonts are compiled
# into the static executable so the guest needs no host resource directories.
source_dir=${DOCSERV_GS_SOURCE:?set DOCSERV_GS_SOURCE to the extracted Ghostscript source}
doc_cc=${DOCSERV_CC:-x86_64-linux-musl-gcc}
doc_cxx=${DOCSERV_CXX:-x86_64-linux-musl-g++}
doc_ar=${DOCSERV_AR:-x86_64-linux-musl-ar}
project_dir=$(pwd)
(
    cd "$source_dir"
    env CC="$doc_cc" CXX="$doc_cxx" AR="$doc_ar" LDFLAGS='-static -no-pie' \
        ./configure --host=x86_64-linux-musl --without-x --disable-cups \
        --disable-dbus --disable-gtk --disable-fontconfig --without-tesseract \
        --without-libpaper --without-libidn --with-drivers=PS --enable-compile-inits
    make -j"${DOCSERV_BUILD_JOBS:-4}"
    cp bin/gs "$project_dir/target/docserv-gs"
)
bash tools/stamp_helios_elf.sh target/docserv-gs
