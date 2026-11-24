#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
work="$root/target/git-helios"
mkdir -p "$work"
cd "$work"

git_version=2.56.1
zlib_version=1.4
fetch() {
    local url=$1 checksum=$2 archive=${1##*/}
    if [[ ! -f "$archive" ]]; then
        curl --fail --location --silent --show-error "$url" --output "$archive.part"
        mv -- "$archive.part" "$archive"
    fi
    printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check -
}

fetch "https://www.kernel.org/pub/software/scm/git/git-$git_version.tar.xz" \
    f0b0bf89264b9bd2e1c640363e7529643989da76281600473c50745cd34a607f
fetch "https://zlib.net/fossils/zlib-$zlib_version.tar.xz" \
    4f11c512053d7bab8d9e9b89bde3575add3e4c50a383b42ef6cb837035fc5a7c

build=$(mktemp -d "$work/build.XXXXXXXX")
trap 'rm -rf -- "$build"' EXIT
tar -xf "zlib-$zlib_version.tar.xz" -C "$build"
tar -xf "git-$git_version.tar.xz" -C "$build"
compiler=${MUSL_CC:-musl-gcc}
jobs=${JOBS:-$(getconf _NPROCESSORS_ONLN)}
(
    cd "$build/zlib-$zlib_version"
    CC="$compiler" ./configure --static --prefix="$build/prefix"
    make -j "$jobs" install
)
make -C "$build/git-$git_version" -j "$jobs" git \
    CC="$compiler" CFLAGS='-O2 -fno-pie' LDFLAGS='-static -no-pie' \
    prefix=/usr ZLIB_PATH="$build/prefix" \
    NO_CURL=YesPlease NO_EXPAT=YesPlease NO_OPENSSL=YesPlease \
    NO_GETTEXT=YesPlease NO_ICONV=YesPlease NO_PCRE2=YesPlease NO_REGEX=YesPlease \
    NO_PERL=YesPlease NO_PYTHON=YesPlease NO_TCLTK=YesPlease NO_RUST=YesPlease \
    FSMONITOR_DAEMON_BACKEND= FSMONITOR_OS_SETTINGS=
"${STRIP:-strip}" -o "$build/git" "$build/git-$git_version/git"
bash "$root/tools/stamp_helios_elf.sh" "$build/git"
mv -- "$build/git" "$work/git"
readelf -h -l -d "$work/git"
sha256sum "$work/git"
