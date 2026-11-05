#!/usr/bin/env bash
set -euo pipefail

# Focused guest gate. Build docserv with tools/build_docserv.sh first.
supervisor_features=document-service-gate
if [[ "${DOCSERV_TEST_UI:-0}" == 1 ]]; then supervisor_features=document-ui-gate; fi
if [[ "${DOCSERV_TEST_UI:-0}" == 1 ]]; then
    bash tools/build_svg_apps.sh -p sunlight-files --features document-thumbnail-smoke
fi
env RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
    cargo build -p sunlightd --release --features "$supervisor_features"
kernel_features=key_inject,document_service
if [[ "${DOCSERV_BACKEND:-hayro}" == hayro ]]; then kernel_features+=,native_document_service; fi
if [[ "${DOCSERV_TEST_POSTSCRIPT:-0}" == 1 ]]; then kernel_features+=,document_postscript; fi
# The UI gate explicitly includes the optional viewer.
injection_phase=document-service
if [[ "${DOCSERV_TEST_UI:-0}" == 1 ]]; then
    injection_phase=desktop_login
    kernel_features+=,document_viewer
    if [[ "${DOCVIEW_BACKEND:-helios}" == native ]]; then kernel_features+=,native_document_viewer; fi
fi
env SUNLIGHT_INJECT_PHASE="$injection_phase" \
    cargo build -p sunlight-kernel --bin sunlight-kernel --features "$kernel_features"
stage=target/docserv-iso-root
mkdir -p "$stage/boot/limine" "$stage/EFI/BOOT"
cp target/x86_64-unknown-none/debug/sunlight-kernel "$stage/boot/sunlight-kernel.elf"
cp limine.conf "$stage/boot/limine/limine.conf"
cp target/limine/bin/limine-bios.sys target/limine/bin/limine-bios-cd.bin \
    target/limine/bin/limine-uefi-cd.bin "$stage/boot/limine/"
cp target/limine/bin/BOOTX64.EFI "$stage/EFI/BOOT/"
xorriso -as mkisofs -R -r -J -b boot/limine/limine-bios-cd.bin \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    --efi-boot boot/limine/limine-uefi-cd.bin -efi-boot-part \
    --efi-boot-image --protective-msdos-label "$stage" -o target/docserv-test.iso
target/limine/bin/limine bios-install target/docserv-test.iso
if [[ "${DOCSERV_SKIP_BOOT:-0}" != 1 ]]; then python3 tools/docserv_boot_gate.py; fi
