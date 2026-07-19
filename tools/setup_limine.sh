#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 LIMINE_DIR [LIMINE_BRANCH]" >&2
    exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LIMINE_DIR="$1"
LIMINE_BRANCH="${2:-v8.x}"
FREESTANDING_SHA256="b280df87c6db0f6ca1dd0a48579e694b403cb0fc77cf6df1e2ddbe69a134b405"
STB_COMMIT="5c205738c191bcb0abc65c4febfa9bd25ff35234"
STB_SHA256="594c2fe35d49488b4382dbfaec8f98366defca819d916ac95becf3e75f4200b3"

has_hash() {
    [[ -f "$1" ]] && [[ "$(sha256sum "$1" | cut -d' ' -f1)" == "$2" ]]
}

limine_is_ready() {
    [[ -x "$LIMINE_DIR/bin/limine" ]] &&
        [[ -f "$LIMINE_DIR/bin/limine-bios.sys" ]] &&
        [[ -f "$LIMINE_DIR/bin/limine-bios-cd.bin" ]] &&
        [[ -f "$LIMINE_DIR/bin/limine-uefi-cd.bin" ]] &&
        [[ -f "$LIMINE_DIR/bin/BOOTX64.EFI" ]]
}

seed_bootstrap_files() {
    local freestanding_target="$LIMINE_DIR/build-aux/freestanding-toolchain"
    local stb_target="$LIMINE_DIR/common/lib/stb_image.h.nopatch"
    local stb_cache="$PROJECT_ROOT/target/limine-bootstrap-cache/stb"

    if ! has_hash "$SCRIPT_DIR/freestanding-toolchain" "$FREESTANDING_SHA256"; then
        echo "[limine] Vendored freestanding-toolchain checksum mismatch." >&2
        exit 1
    fi

    mkdir -p "$(dirname "$freestanding_target")"
    cp "$SCRIPT_DIR/freestanding-toolchain" "$freestanding_target"
    chmod +x "$freestanding_target"

    if has_hash "$stb_target" "$STB_SHA256"; then
        return
    fi

    echo "[limine] Fetching stb_image.h through git (avoids raw.githubusercontent.com)..."
    if [[ ! -d "$stb_cache/.git" ]]; then
        rm -rf "$stb_cache"
        mkdir -p "$stb_cache"
        git -C "$stb_cache" init -q
        git -C "$stb_cache" remote add origin https://github.com/nothings/stb.git
    fi
    git -C "$stb_cache" fetch --depth=1 origin "$STB_COMMIT"
    mkdir -p "$(dirname "$stb_target")"
    git -C "$stb_cache" show "$STB_COMMIT:stb_image.h" >"$stb_target"

    if ! has_hash "$stb_target" "$STB_SHA256"; then
        rm -f "$stb_target"
        echo "[limine] stb_image.h checksum mismatch." >&2
        exit 1
    fi
}

if limine_is_ready; then
    echo "[limine] Limine already built (BIOS + UEFI CD artifacts present)."
    exit 0
fi

if ! command -v mtools >/dev/null 2>&1 || ! command -v mformat >/dev/null 2>&1; then
    echo "[limine] mtools is required to build limine-uefi-cd.bin (hybrid UEFI ISO)." >&2
    echo "[limine] Install mtools (e.g. apt install mtools) and retry." >&2
    exit 1
fi

if [[ ! -d "$LIMINE_DIR" ]]; then
    echo "[limine] Cloning Limine..."
    git clone --branch="$LIMINE_BRANCH" --depth=1 \
        https://github.com/limine-bootloader/limine.git "$LIMINE_DIR"
elif [[ ! -d "$LIMINE_DIR/.git" ]]; then
    echo "[limine] Incomplete non-git directory at $LIMINE_DIR; remove it and retry." >&2
    exit 1
else
    echo "[limine] Resuming incomplete Limine setup (or adding missing UEFI CD artifact)..."
fi

seed_bootstrap_files

pushd "$LIMINE_DIR" >/dev/null
if [[ ! -x ./configure ]]; then
    ./bootstrap
fi
# Rebuild with UEFI CD support so hybrid ISO generation matches Limine USAGE.md.
./configure --enable-uefi-x86-64 --enable-bios-cd --enable-bios-pxe --enable-uefi-cd
make -j"$(nproc)"
popd >/dev/null

if ! limine_is_ready; then
    echo "[limine] Build completed without the required BIOS/UEFI boot files." >&2
    ls -la "$LIMINE_DIR/bin" 2>/dev/null || true
    exit 1
fi

echo "[limine] Limine built successfully (BIOS + UEFI)."
