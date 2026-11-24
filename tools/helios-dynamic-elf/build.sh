#!/usr/bin/env bash
set -euxo pipefail
cd "$(dirname "$0")/../.."
out="$PWD/target/helios-dynamic-elf"
features=helios_dynamic_elf_probe
launcher_features=dynamic-elf-probe
variants=(pie exec static)
if [[ ${1:-} == handoff ]]; then
    out="$PWD/target/helios-dynamic-elf-handoff/${2:?run name required}"
    features=helios_dynamic_elf_handoff
fi
if [[ ${1:-} == relocation || ${1:-} == waitpid ]]; then
    out="$PWD/target/helios-dynamic-elf-relocation/${2:?run name required}"
    if [[ $1 == waitpid ]]; then
        out="$PWD/target/helios-dynamic-elf-waitpid/$2"
    fi
    features=helios_dynamic_elf_relocation_fixture
    launcher_features=dynamic-elf-relocation-fixture
    variants+=(relocation)
fi
mkdir -p "$out/bundles"
exec > >(tee "$out/build.log") 2>&1
source_file=tools/helios-dynamic-elf/probe.c
for variant in "${variants[@]}"; do
    bundle="$out/bundles/elf-$variant.sunapp"
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
executable = "Program/probe"
EOF
    case "$variant" in
        pie|relocation) flags=(-fPIE -pie) ;;
        exec) flags=(-fno-pie -no-pie) ;;
        static) flags=(-fno-pie -no-pie -static) ;;
    esac
    elf="$bundle/Program/probe"
    if [[ $variant == relocation ]]; then
        gcc -nostdlib "${flags[@]}" -Wl,--hash-style=sysv,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            tools/helios-dynamic-elf/relocation-start.S -o "$elf"
    elif [[ $features != helios_dynamic_elf_probe && $variant != static ]]; then
        if [[ $variant == exec ]]; then flags+=(-DFIXED_EXEC); fi
        gcc -nostdlib "${flags[@]}" -Wl,--dynamic-linker=/lib/helios/ld-handoff.so.1 \
            tools/helios-dynamic-elf/handoff-start.S -o "$elf"
    else
        musl-gcc -Os "${flags[@]}" "$source_file" -o "$elf"
    fi
    bash tools/stamp_helios_elf.sh "$elf"
    readelf -W -h -l -d -r "$elf" >"$out/$variant.readelf"
    if [[ $features == helios_dynamic_elf_handoff && $variant != static ]]; then
        grep -F 'Requesting program interpreter: /lib/helios/ld-handoff.so.1' "$out/$variant.readelf"
    fi
done
interpreter=$(sed -n 's/.*Requesting program interpreter: \(.*\)]/\1/p' "$out/pie.readelf")
if [[ $features != helios_dynamic_elf_probe ]]; then
    printf '%s\n' "$interpreter" >"$out/interpreter-identity.txt"
    interpreter="$out/ld-handoff.so.1"
    gcc -Os -fPIC -shared -nostdlib -ffreestanding -fno-stack-protector -Wl,-e,_start \
        tools/helios-dynamic-elf/handoff-interpreter.c -o "$interpreter"
    bash tools/stamp_helios_elf.sh "$interpreter"
fi
readelf -W -h -l -d -r "$interpreter" >"$out/interpreter.readelf"
find "$out/bundles" -type f | sort >"$out/bundle-contents.txt"
cat "$out"/bundles/*/Manifest.toml >"$out/manifests.txt"
sha256sum "$interpreter" "$out"/bundles/*/{Manifest.toml,Program/probe} >"$out/SHA256SUMS"
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
    cargo build -p helios-wayland-fixture --bin dynamic-elf-probe --release --features "$launcher_features"
launcher="$PWD/target/x86_64-unknown-none/release/dynamic-elf-probe"
SUNLIGHT_DYNAMIC_PROBES="$out/bundles" SUNLIGHT_DYNAMIC_PROBE_LAUNCHER="$launcher" \
    SUNLIGHT_HELIOS_HANDOFF_INTERPRETER="$interpreter" \
    cargo build -p sunlight-kernel --features "$features"
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel "$out/guest.iso"
sha256sum "$launcher" "$out/guest.iso" >>"$out/SHA256SUMS"
