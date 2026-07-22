#!/usr/bin/env bash
set -euo pipefail

proof_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
proof_binary="$proof_root/target/mem-string-proof"
codegen_object="$proof_root/target/mem-string-codegen.o"
c_abi_object="$proof_root/target/mem-string-abi.o"
c_abi_linked_object="$proof_root/target/mem-string-abi-linked.o"
cpp_abi_object="$proof_root/target/mem-string-abi-cpp.o"

mkdir -p "$proof_root/target"
rustc --test "$proof_root/tools/mem-string-proof.rs" -o "$proof_binary"
"$proof_binary" --test-threads=1

# Compile the real exports without the host-test cfg. The byte loops must not
# lower into calls back to symbols that this libc itself owns.
rustc "$proof_root/tools/mem-string-codegen.rs" \
    --crate-name sunlight_libc_mem_string_codegen \
    --crate-type lib \
    --edition 2021 \
    --target x86_64-unknown-none \
    -C panic=abort \
    -O \
    --emit=obj \
    -o "$codegen_object"

required_symbols='memcpy|memmove|memset|memcmp|memchr|strlen|strnlen|strcmp|strncmp|strchr|strrchr'
for tool in llvm-nm llvm-readelf; do
    command -v "$tool" >/dev/null
done

for symbol in memcpy memmove memset memcmp memchr strlen strnlen strcmp strncmp strchr strrchr; do
    llvm-nm -g --defined-only "$codegen_object" | grep -Eq " [Tt] ${symbol}$"
done
if llvm-nm -u "$codegen_object" | grep -Eq " ${required_symbols}$"; then
    echo "memory/string codegen has an unexpected primitive dependency" >&2
    exit 1
fi
if llvm-readelf -rW "$codegen_object" | grep -Eq "${required_symbols}"; then
    echo "memory/string codegen relocates to one of its own primitives" >&2
    exit 1
fi

# There is no host linker involved: compile a freestanding C11 consumer using
# the public header, then resolve its calls against the exact object audited
# above. A remaining undefined primitive means the header and exported ABI
# diverged.
clang -target x86_64-unknown-none \
    -std=c11 \
    -ffreestanding \
    -fno-builtin \
    -I "$proof_root/sunlight-libc/include" \
    -c "$proof_root/tools/mem-string-abi.c" \
    -o "$c_abi_object"
ld.lld -r "$codegen_object" "$c_abi_object" -o "$c_abi_linked_object"
if llvm-nm -u "$c_abi_linked_object" | grep -Eq " ${required_symbols}$"; then
    echo "C ABI probe left a memory/string primitive unresolved" >&2
    exit 1
fi

# The header intentionally remains consumable from C++ as well.
clang++ -target x86_64-unknown-none \
    -std=c++17 \
    -ffreestanding \
    -fno-builtin \
    -I "$proof_root/sunlight-libc/include" \
    -c "$proof_root/tools/mem-string-abi.cpp" \
    -o "$cpp_abi_object"
