# Phase 22 local-static guard acceptance (2026-11-12)

Fresh evidence for the bounded native C++ subset. Commands were run from the
repository root. `2cpu/` and `4cpu/` contain the full native suite results
(93 buildable cases PASS plus the preserved original historical LINK_FAIL row).

```sh
bash tools/build_native_cxx.sh
python3 tools/native-c/tests/run_cxx_runtime.py --out NEW-runtime
python3 tools/native-c/tests/run_cxx_classes.py --out NEW-classes
python3 tools/native-c/tests/run_cxx_output.py --out NEW-output
python3 tools/native-c/tests/run_cxx_string.py --out NEW-string
python3 tools/native-c/tests/run_cxx_exception_discovery.py --out NEW-discovery
python3 tools/native-c/guest.py --suite --cpus 2 --timeout 180 --out NEW-2cpu
python3 tools/native-c/guest.py --suite --cpus 4 --timeout 180 --out NEW-4cpu
python3 tools/native-c/cxx.py tools/native-c/tests/cxx-lifecycle.cpp \
  -Wl,--wrap=malloc,--wrap=free -o NEW-lifecycle.elf
python3 tools/native-c/guest.py NEW-lifecycle.elf --cpus 2 --out NEW-life-2cpu
python3 tools/native-c/guest.py NEW-lifecycle.elf --cpus 4 --out NEW-life-4cpu
python3 tools/native-c/cxx.py docs/audit/probes/cpp-runtime.cpp -o NEW-historical.elf
python3 tools/native-c/guest.py NEW-historical.elf --cpus 2 --expect-exit 0 --out NEW-hist-2cpu
python3 tools/native-c/guest.py NEW-historical.elf --cpus 4 --expect-exit 0 --out NEW-hist-4cpu
```

The original archive-free command is preserved verbatim and executed only for
diagnostics:

```sh
clang++ --target=x86_64-unknown-none-elf -nostdinc \
  -I"$PWD/sunlight-libc/include" -nostdlib -fuse-ld=lld \
  docs/audit/probes/cpp-runtime.cpp -o NEW-original.elf
```

`historical-original.log` records its LINK_FAIL and 20 unresolved symbols.
`guard-object-inspection.log` records the compiler-emitted 8-byte guard
objects, `__cxa_guard_*`/`__cxa_atexit` relocations, and the native futex
syscall path in the final ELF.
