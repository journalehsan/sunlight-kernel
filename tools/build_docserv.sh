#!/usr/bin/env bash
set -euo pipefail

# Hayro remains host-only. Never build/stamp it as a Helios guest renderer.
if [[ "${DOCSERV_BACKEND:-hayro}" != mupdf-legacy ]]; then
    echo "Hayro is host-only; native PDF rendering is not available yet." >&2
    echo "For temporary MuPDF rollback, set DOCSERV_BACKEND=mupdf-legacy." >&2
    exit 1
fi

# Supply a musl C/C++ cross toolchain. MuPDF is C-based userspace and must not
# inherit the kernel's bare-metal linker script or link against host glibc.
doc_cc=${DOCSERV_CC:-x86_64-linux-musl-gcc}
doc_cxx=${DOCSERV_CXX:-x86_64-linux-musl-g++}
doc_ar=${DOCSERV_AR:-x86_64-linux-musl-ar}
for tool in "$doc_cc" "$doc_cxx" "$doc_ar"; do
    command -v "$tool" >/dev/null || { echo "missing musl tool: $tool" >&2; exit 1; }
done
features=()
if [[ -n "${DOCSERV_FEATURES:-}" ]]; then features=(--features "$DOCSERV_FEATURES"); fi
env CC="$doc_cc" CXX="$doc_cxx" AR="$doc_ar" \
    RUSTFLAGS="-C linker=$doc_cc -C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie" \
    cargo --config tools/docserv-mupdf-legacy.toml build --no-default-features --features sunlight-docserv/mupdf-legacy,sunlight-docview/std -p sunlight-docserv -p sunlight-docview --release --target x86_64-unknown-linux-musl "${features[@]}"
bash tools/stamp_helios_elf.sh target/x86_64-unknown-linux-musl/release/sunlight-docserv
bash tools/stamp_helios_elf.sh target/x86_64-unknown-linux-musl/release/sunlight-docview
