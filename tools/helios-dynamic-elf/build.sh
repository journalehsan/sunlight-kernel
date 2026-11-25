#!/usr/bin/env bash
set -euxo pipefail
cd "$(dirname "$0")/../.."
allocator_mode=
malloc_mode=
dso_mode=
if [[ ${1:-} == c-dso ]]; then dso_mode=1; fi
startup_mode=
if [[ ${1:-} == c-startup || -n $dso_mode ]]; then startup_mode=1; fi
if [[ ${1:-} == malloc || -n $startup_mode ]]; then malloc_mode=1; allocator_mode=1; fi
mmap_run=
if [[ ${1:-} == allocator ]]; then allocator_mode=1; fi
if [[ ${1:-} == mmap || -n $allocator_mode ]]; then
    mmap_run=${2:?run name required}
    set -- tls "$mmap_run"
fi
out="$PWD/target/helios-dynamic-elf"
features=helios_dynamic_elf_probe
launcher_features=dynamic-elf-probe
variants=(pie exec static)
if [[ ${1:-} == handoff ]]; then
    out="$PWD/target/helios-dynamic-elf-handoff/${2:?run name required}"
    features=helios_dynamic_elf_handoff
fi
if [[ ${1:-} == relocation || ${1:-} == waitpid || ${1:-} == shared || ${1:-} == real-shared || ${1:-} == dependency-graph || ${1:-} == slot-reuse || ${1:-} == lifecycle || ${1:-} == tls ]]; then
    out="$PWD/target/helios-dynamic-elf-relocation/${2:?run name required}"
    if [[ $1 == waitpid ]]; then
        out="$PWD/target/helios-dynamic-elf-waitpid/$2"
    fi
    features=helios_dynamic_elf_relocation_fixture
    launcher_features=dynamic-elf-relocation-fixture
    variants+=(relocation)
fi
if [[ ${1:-} == shared || ${1:-} == real-shared || ${1:-} == dependency-graph || ${1:-} == slot-reuse || ${1:-} == lifecycle || ${1:-} == tls ]]; then
    out="$PWD/target/helios-dynamic-elf-shared-library/${2:?run name required}"
    if [[ $1 == real-shared ]]; then out="$PWD/target/helios-dynamic-elf-real-shared/$2"; fi
    features=helios_dynamic_elf_shared_fixture
    launcher_features=dynamic-elf-shared-fixture
    variants+=(shared real graph)
    if [[ $1 == dependency-graph ]]; then
        out="$PWD/target/helios-dynamic-elf-dependency-graph/$2"
    fi
    if [[ $1 == slot-reuse ]]; then
        out="$PWD/target/helios-dynamic-elf-slot-reuse/$2"
    fi
fi
if [[ ${1:-} == lifecycle ]]; then
    out="$PWD/target/helios-process-lifecycle-complete/$2"
    features=helios_dynamic_elf_orphan_fixture
    launcher_features=dynamic-elf-orphan-fixture
fi
if [[ ${1:-} == tls ]]; then
    out="$PWD/target/helios-dynamic-elf-tls-bss/${2:?run name required}"
    features=helios_dynamic_elf_tls_fixture
    launcher_features=dynamic-elf-tls-fixture
    variants+=(tls tls-init tls-shared)
    out="$PWD/target/helios-dynamic-elf-tls-shared/${2:?run name required}"
fi
if [[ -n $mmap_run ]]; then
    out="$PWD/target/helios-runtime-mmap/$mmap_run"
    features=helios_runtime_mmap
    launcher_features=runtime-mmap
    variants+=(mmap)
    if [[ -n $allocator_mode ]]; then
        out="$PWD/target/helios-runtime-allocator/$mmap_run"
        features=helios_runtime_page_allocator
        launcher_features+=,runtime-page-allocator
    fi
fi
if [[ -n $malloc_mode ]]; then
    out="$PWD/target/helios-runtime-malloc/$mmap_run"
    launcher_features+=,runtime-malloc
fi
if [[ -n $startup_mode ]]; then
    out="$PWD/target/helios-runtime-c-startup/$mmap_run"
    features=helios_runtime_c_startup
    launcher_features+=,runtime-c-startup
    variants+=(c-startup)
fi
if [[ -n $dso_mode ]]; then
    out="$PWD/target/helios-runtime-c-dso/$mmap_run"
    features=helios_runtime_c_dso
    launcher_features+=,runtime-c-dso
    variants+=(c-dso)
fi
mkdir -p "$out/bundles"
exec > >(tee "$out/build.log") 2>&1
{ gcc --version; gcc -dumpmachine; "$(gcc -print-prog-name=ld)" --version; } >"$out/toolchain.txt"
source_file=tools/helios-dynamic-elf/probe.c
allocator_flags=()
if [[ -n $allocator_mode ]]; then allocator_flags+=(-DPAGE_ALLOCATOR_FIXTURE); fi
if [[ -n $malloc_mode ]]; then allocator_flags+=(-DMALLOC_FIXTURE); fi
for variant in "${variants[@]}"; do
    bundle="$out/bundles/elf-$variant.sunapp"
    entry=probe
    if [[ $variant == mmap ]]; then entry=mmap-probe; fi
    mkdir -p "$bundle/Program"
    cat >"$bundle/Manifest.toml" <<EOF
[bundle]
format = 1
[application]
id = "org.sunlight.helios.elf-$variant"
name = "ELF $variant probe"
kind = "gui"
[runtime]
type = "helios"
[entry]
executable = "Program/$entry"
EOF
    case "$variant" in
        pie|relocation|shared|real|graph|tls|tls-init|tls-shared|c-dso) flags=(-fPIE -pie) ;;
        exec) flags=(-fno-pie -no-pie) ;;
        static|mmap|c-startup) flags=(-fno-pie -no-pie -static) ;;
    esac
    elf="$bundle/Program/probe"
    if [[ $variant == mmap ]]; then
        gcc -Os -Wall -Wextra -Werror -ffreestanding -fno-stack-protector -nostdlib \
            "${flags[@]}" "${allocator_flags[@]}" tools/helios-dynamic-elf/anon-mmap.c -o "$elf"
    elif [[ $variant == c-dso ]]; then
        utility="$bundle/Program/libhelios-util.so"
        c_flags=(-Os -Wall -Wextra -Werror -ffreestanding -fno-stack-protector -fno-asynchronous-unwind-tables -fno-unwind-tables -nostdlib)
        gcc -v "${c_flags[@]}" -fPIC -shared -Wl,--hash-style=sysv,-z,norelro,-z,defs,-soname,libhelios-util.so \
            tools/helios-dynamic-elf/c-utility.c -o "$utility"
        readelf -W -h -l -d -r -s -S -V "$utility" >"$out/c-utility.readelf"
        python3 tools/helios-dynamic-elf/inspect_c_utility.py "$out/c-utility.readelf"
        gcc -v "${c_flags[@]}" "${flags[@]}" -mno-red-zone -DHELIOS_C_DSO \
            -Wl,-e,_start,--hash-style=sysv,-z,norelro,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            tools/helios-dynamic-elf/{c-startup-entry.S,c-startup.c} "$utility" -o "$elf"
    elif [[ $variant == c-startup ]]; then
        gcc -Os -Wall -Wextra -Werror -ffreestanding -fno-stack-protector -mno-red-zone \
            -nostdlib "${flags[@]}" -Wl,-e,_start \
            tools/helios-dynamic-elf/{c-startup-entry.S,c-startup.c} -o "$elf"
    elif [[ $variant == shared ]]; then
        library="$bundle/Program/libhelios-value.so"
        gcc -Os -fPIC -shared -nostdlib -Wl,--hash-style=sysv,-z,norelro,-soname,libhelios-value.so,-e,helios_fixture_value \
            tools/helios-dynamic-elf/shared-value.c -o "$library"
        bash tools/stamp_helios_elf.sh "$library"
        readelf -W -h -l -d -r -s "$library" >"$out/library.readelf"
        gcc -Os -ffreestanding -nostdlib "${flags[@]}" -Wl,--hash-style=sysv,-z,norelro,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            tools/helios-dynamic-elf/shared-start.c "$library" -o "$elf"
    elif [[ $variant == real ]]; then
        real_library="$bundle/Program/libhelios-math.so"
        gcc -Os -fPIC -shared -nostdlib -Wl,--hash-style=sysv,-z,norelro,-soname,libhelios-math.so \
            tools/helios-dynamic-elf/real-value.c -o "$real_library"
        readelf -W -h -l -d -r -s "$real_library" >"$out/real-library.readelf"
        gcc -Os -ffreestanding -nostdlib "${flags[@]}" -DREAL_SHARED \
            -Wl,--hash-style=sysv,-z,norelro,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            tools/helios-dynamic-elf/shared-start.c "$real_library" -o "$elf"
    elif [[ $variant == graph ]]; then
        graph_a="$bundle/Program/libhelios_a.so"
        graph_b="$bundle/Program/libhelios_b.so"
        gcc -Os -fPIC -shared -nostdlib -Wl,--hash-style=sysv,-z,norelro,-soname,libhelios_b.so \
            tools/helios-dynamic-elf/graph-b.c -o "$graph_b"
        gcc -Os -fPIC -shared -nostdlib -Wl,--hash-style=sysv,-z,norelro,-soname,libhelios_a.so \
            tools/helios-dynamic-elf/graph-a.c "$graph_b" -o "$graph_a"
        gcc -Os -ffreestanding -nostdlib "${flags[@]}" -DGRAPH_SHARED \
            -Wl,--hash-style=sysv,-z,norelro,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            -Wl,-rpath-link,"$bundle/Program" tools/helios-dynamic-elf/shared-start.c "$graph_a" -o "$elf"
        readelf -W -h -l -d -r -s "$graph_a" >"$out/graph-a.readelf"
        readelf -W -h -l -d -r -s "$graph_b" >"$out/graph-b.readelf"
    elif [[ $variant == tls-shared ]]; then
        tls_library="$bundle/Program/libhelios_tls.so"
        gcc -Os -fPIC -ffreestanding -fno-stack-protector -c \
            tools/helios-dynamic-elf/tls-shared-value.c -o "$out/tls-value.o"
        gcc -shared -nostdlib -Wl,--hash-style=sysv,-z,norelro,-soname,libhelios_tls.so \
            "$out/tls-value.o" -o "$tls_library"
        gcc -Os -ffreestanding -fno-stack-protector -ftls-model=local-exec -fPIE -c \
            tools/helios-dynamic-elf/tls-shared-start.c -o "$out/tls-shared.o"
        gcc -pie -nostdlib -Wl,--hash-style=sysv,-z,norelro,--dynamic-linker=/lib/helios/ld-handoff.so.1,--allow-shlib-undefined \
            "$out/tls-shared.o" -L"$bundle/Program" -lhelios_tls -o "$elf"
        readelf -W -h -l -d -r -s "$tls_library" >"$out/tls-library.readelf"
        objdump -drw "$tls_library" >"$out/tls-library.dis"
        objcopy --dump-section ".tdata=$out/dso.tls-image.bin" "$tls_library"
        objcopy --dump-section ".tdata=$out/app.tls-image.bin" "$elf"
    elif [[ $variant == tls || $variant == tls-init ]]; then
        tls_flags=()
        if [[ $variant == tls ]]; then tls_flags+=(-DTLS_BSS); fi
        gcc -Os -ffreestanding -fno-stack-protector -ftls-model=local-exec -fPIE -c \
            "${tls_flags[@]}" tools/helios-dynamic-elf/tls-start.c -o "$out/$variant.o"
        gcc -pie -nostdlib -Wl,--hash-style=sysv,-z,norelro,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            "$out/$variant.o" -o "$elf"
        objdump -dr "$out/$variant.o" >"$out/$variant-object.dis"
        objdump -d "$elf" >"$out/$variant.dis"
        readelf -W -x .tdata "$elf" >"$out/$variant-tdata.readelf"
        grep -F R_X86_64_TPOFF32 "$out/$variant-object.dis"
        grep -F '%fs:0xfffffffffffffffc' "$out/$variant.dis"
        if [[ $variant == tls ]]; then grep -F '%fs:0xfffffffffffffff8' "$out/$variant.dis"; fi
        readelf -Wd "$elf" >"$out/$variant-dynamic.readelf"
        if grep -Eq '\((NEEDED|INIT|FINI|INIT_ARRAY|FINI_ARRAY|PREINIT_ARRAY|RELA|REL|JMPREL|RELR)\)|STATIC_TLS' "$out/$variant-dynamic.readelf"; then
            exit 1
        fi
    elif [[ $variant == relocation ]]; then
        gcc -nostdlib "${flags[@]}" -Wl,--hash-style=sysv,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            tools/helios-dynamic-elf/relocation-start.S -o "$elf"
    elif [[ $features != helios_dynamic_elf_probe && $variant != static ]]; then
        if [[ $variant == exec ]]; then flags+=(-DFIXED_EXEC); fi
        gcc -nostdlib "${flags[@]}" -Wl,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            tools/helios-dynamic-elf/handoff-start.S -o "$elf"
    else
        musl-gcc -Os "${flags[@]}" "$source_file" -o "$elf"
    fi
    if [[ $variant != real && $variant != graph ]]; then bash tools/stamp_helios_elf.sh "$elf"; fi
    readelf -W -h -l -d -r -s "$elf" >"$out/$variant.readelf"
    if [[ $variant == tls || $variant == tls-init ]]; then
        python3 tools/helios-dynamic-elf/inspect_tls.py "$variant" "$elf" "$out/$variant.readelf" >"$out/$variant.json"
    fi
    if [[ $features == helios_dynamic_elf_handoff && $variant != static ]]; then
        grep -F 'Requesting program interpreter: /lib/helios/ld-handoff.so.1' "$out/$variant.readelf"
    fi
done
interpreter=$(sed -n 's/.*Requesting program interpreter: \(.*\)]/\1/p' "$out/pie.readelf")
if [[ $features != helios_dynamic_elf_probe ]]; then
    printf '%s\n' "$interpreter" >"$out/interpreter-identity.txt"
    interpreter="$out/ld-handoff.so.1"
    interp_flags=()
    if [[ -n $dso_mode ]]; then interp_flags+=(-DC_DSO_FIXTURE); fi
    if [[ -n $malloc_mode ]]; then interp_flags+=(-DMALLOC_FIXTURE); fi
    if [[ -n ${tls_library:-} ]]; then
        interp_flags+=(-DSTARTUP_TLS -Wl,--section-start=.tlsresolver=0x4000)
    fi
    gcc -Os -fPIC -shared -nostdlib -ffreestanding -fno-stack-protector -Wl,-e,_start "${interp_flags[@]}" \
        tools/helios-dynamic-elf/handoff-interpreter.c -o "$interpreter"
    bash tools/stamp_helios_elf.sh "$interpreter"
fi
readelf -W -h -l -d -r -s "$interpreter" >"$out/interpreter.readelf"
find "$out/bundles" -type f | sort >"$out/bundle-contents.txt"
cat "$out"/bundles/*/Manifest.toml >"$out/manifests.txt"
sha256sum "$interpreter" "$out"/bundles/*/{Manifest.toml,Program/probe} >"$out/SHA256SUMS"
if [[ -n ${real_library:-} ]]; then
    sha256sum "$library" "$real_library" >>"$out/SHA256SUMS"
    sed -n '/canonical_id: "elf-/,/},/p' sunlight-libc/src/sun_exec.rs >"$out/registry.txt"
fi
if [[ -n ${graph_a:-} ]]; then
    sha256sum "$graph_a" "$graph_b" >>"$out/SHA256SUMS"
fi
if [[ -n ${tls_library:-} ]]; then
    sha256sum "$tls_library" "$out/"*.tls-image.bin >>"$out/SHA256SUMS"
fi
if [[ -n $dso_mode ]]; then sha256sum "$utility" >>"$out/SHA256SUMS"; fi
sha256sum -c "$out/SHA256SUMS"
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
    cargo build -p helios-wayland-fixture --bin dynamic-elf-probe --release --features "$launcher_features"
launcher="$PWD/target/x86_64-unknown-none/release/dynamic-elf-probe"
SUNLIGHT_DYNAMIC_PROBES="$out/bundles" SUNLIGHT_DYNAMIC_PROBE_LAUNCHER="$launcher" \
    SUNLIGHT_HELIOS_HANDOFF_INTERPRETER="$interpreter" \
    cargo build -p sunlight-kernel --features "$features"
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel "$out/guest.iso"
sha256sum "$launcher" target/x86_64-unknown-none/debug/sunlight-kernel "$out/guest.iso" >>"$out/SHA256SUMS"
