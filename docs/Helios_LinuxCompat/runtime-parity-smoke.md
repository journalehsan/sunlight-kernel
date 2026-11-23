# Static musl and native C++ runtime parity smoke

November 22, 2026. Base revision `307420087b67b625ab62591dbd4ab281e21b2645`.
Final run: `target/runtime-parity/acceptance-04/`. Both fresh fixtures ran as
UID 1000, sequentially from the same disposable image, on QEMU 11.1.3 (TCG,
4 vCPUs, 4096 MiB, 120-second replay deadline, no NIC or external data disk).

| Check | Static musl / Helios | Native C++ |
|---|---|---|
| Build, static ELF and ABI inspection | PASS | PASS |
| Exclusive create, five exact bytes, close/reopen, compare, EOF, unlink | PASS | PASS |
| Initialized TLS, two overlapping workers, distinct values, main unchanged | PASS | PASS |
| Worker return and successful join, process status 0, clean guest reap | PASS | PASS |
| Loader personality, successful kernel VFS opens, worker admission | PASS | PASS |

The file lives in the guest's existing writable `/tmp`, under a distinct
fixture-owned name and mode 0600. An atomic rendezvous holds both workers live
after their TLS writes while main reads its own value. Each worker rereads
its value before returning; the main thread checks both completion values
after joining. Musl also checks each returned pthread pointer; C++ checks
both joined handles are no longer joinable.

Run from the repository root with a fresh output directory:

```sh
tools/runtime-parity/run.sh target/runtime-parity/acceptance-05
```

The final run used these commands (with absolute paths in the recorded logs):

```sh
out="$PWD/target/runtime-parity/acceptance-04"
bash -x tools/build_native_cxx.sh
musl-gcc -v -std=c11 -O2 -Wall -Wextra -Werror -static -fno-pie -no-pie -pthread \
  "-Wl,-Map,$out/musl.map" tools/runtime-parity/musl.c -o "$out/musl.elf"
bash tools/stamp_helios_elf.sh "$out/musl.elf"
python3 tools/native-c/cxx.py -v -O2 -Wall -Wextra -Werror \
  "-Wl,-Map,$out/native.map,--why-extract=$out/native-extract.tsv" \
  tools/runtime-parity/native.cpp -o "$out/native.elf"
for fixture in musl native; do
  readelf -W -h -l -S -d -s "$out/$fixture.elf" > "$out/$fixture.readelf"
  objdump -d "$out/$fixture.elf" > "$out/$fixture.disassembly"
done
python3 tools/runtime-parity/verify.py prepare "$out"
python3 tools/native-c/guest.py --cases "$out/cases.json" --mixed-abi --memory-mb 4096 \
  --cpus 4 --timeout 120 --out "$out/guest"
python3 tools/runtime-parity/verify.py trace "$out"
```

`guest/{support-build,kernel-build,iso-build,qemu}.log` records the underlying
commands. `run.log`, the verbose link logs, maps, extraction report, ELF dumps,
serial and stream captures hold the full evidence.

| Artifact | SHA-256 |
|---|---|
| `musl.elf` | `f26ff41f3e483d5bf8ce8953bd467e0ab0575f58ab0270cf5f3e852ddaba7e61` |
| `native.elf` | `5b317f696f88edf6f854e287fcd7b598d9b19fd4bd58cf24a340a89f152f1a2d` |
| `guest/kernel.elf` | `b08c2929d9a7847973bb722cab58ae99ec3fa7b9977422ae6684eccdcdfd8c42` |
| `guest/guest.iso` | `ee183343019977a249cf353e7a3ce9b62b59505489f5f17f0622668de475c8e5` |
| `libsunlight_cxx.a` | `1aee1754b50a3976e3bf8cd927f1255b08fd7fb2f277461178936d52db92588e` |
| `libsunlight_eh.a` | `d24bf569de87cb06ae10259d07a6ae7a401bad331bc3b699bd8cd49859627d31` |
| `libsunlight_native_c.a` | `35f5137be5baee2b14591aa75417670f2b4c07ce71a53ee4ac291ffcd8b448cf` |

Both ELFs were x86-64 ET_EXEC without PT_INTERP, PT_DYNAMIC or DT_NEEDED.
Musl had OSABI 3 and `HLNX01`, a four-byte initialized PT_TLS (align 4),
and no native bridge. The link map shows `open.lo`, `__init_tls.lo`,
`pthread_create.lo` and `pthread_join.lo` from `/usr/lib/musl/lib/libc.a`
(musl 1.2.6-2). This uses the static non-PIE layout from `tools/build.sh`
and the repository Helios stamp.

Native had OSABI 0 with no Linux stamp. Its PT_TLS had four file bytes and
32 memory bytes (align 8), with `.tdata` and `.tbss` and a four-byte TLS
symbol for the fixture. The Phase 22 native contract uses the linker symbols
`__tls_tdata_start`, `__tls_tdata_end`, `__tls_tbss_end`, and `__tls_align`
in `sunlight-libc/src/tls.rs` to copy and zero the static image and set up
the thread pointer. The final disassembly loads the pointer from FS:0 and
accesses the fixture value at pointer minus 0x20. The presence of PT_TLS
does not imply musl startup semantics. The verbose link line lists all
three native archives. The LLD extraction report counts 4 C++ members,
10 EH members and 52 native C members; the retained map includes
`exception.o`, EH `entry.o` and `UnwindLevel1.c.o`, and native libc
sections for `open`, `pthread_create` and `pthread_join`. Extraction
and retention are link evidence; neither shows that unwinding ran.

Guest observations, with repeated identical opens shown once per fixture:

```text
[NATIVE-C-START] case=0 name=parity-musl pid=100 bytes=28520 owned_frames=519 uid=1000 abi=linux
[PARITY-OPEN] pid=100 linux=true nr=2 handler=sys_open backend=kernel-vfs
[HELIOS-THREAD] clone tgid=100 tid=101 tls=0x1000022f38 stack=0x1000022ee8
[HELIOS-THREAD] clone tgid=100 tid=102 tls=0x1000045f38 stack=0x1000045ee8
[HELIOS-THREAD] clear_child_tid tid=102 addr=0x4062b0 woken=1
[HELIOS-THREAD] clear_child_tid tid=101 addr=0x4062b0 woken=1
[NATIVE-C-START] case=1 name=parity-native pid=101 bytes=184216 owned_frames=548 uid=1000 abi=native
[PARITY-OPEN] pid=101 linux=false nr=40 handler=sys_open backend=kernel-vfs
[SYSCALL] thread_spawn: parent=101 tid=103 trampoline=0x410190
[SYSCALL] thread_spawn: parent=101 tid=104 trampoline=0x410190
[NATIVE-C-DONE] cases=2
```

Both captured streams contained exactly the fixture-prefixed `START`,
`FILE PASS`, `TLS PASS` and `JOIN PASS` lines, with empty stderr. Guest result
rows reported exit 0 and `clean=1`; QEMU returned the runner's completion
status 33 with no recorded kernel fault. The scoped open trace fires after
kernel VFS descriptor installation and excludes the service open path.
Source maps Linux open 2 to kernel open 40 and Linux clone 56 to
`sys_linux_clone`. Native libc uses syscall 22 for this worker spawn.
The worker-admission traces observe those handlers; join success is observed
through the fixture assertions, not a native join syscall trace. PID 101 is
reused across the sequential cases. A zero-frame worker reap line carrying
`case=1` in the first case is not the later native leader's reap; the leader
reap has positive user and page-table frame counts.

Earlier attempts: the stale native C lockfile blocked the first archive build.
Replay 01 stopped at a host verifier that did not recognize the mangled native
TLS symbol; replay 02 stopped at the runner's Linux marker guard. Replay 03
stalled in the test trace after Linux create reached VFS: the trace attempted
to acquire the scheduler lock a second time. No guest checks passed in those
attempts. Replay 04 uses the existing guard and passed without changing ABI
routing, runtime support, filesystem permissions or fixture assertions.

**NOT RUN:** explicit `pthread_exit`, cancellation, nontrivial TLS destructors,
exception unwinding, dynamic musl, ld-linux, shared objects, static glibc, and
broader compatibility testing. The existing `helios-note` binary was not used
as evidence.

Changed paths: `tools/runtime-parity/{checks.h,musl.c,native.cpp,run.sh,verify.py}`,
`tools/native-c/{guest.py,Cargo.lock}`, `kernel/src/native_c_guest.rs`,
`kernel/src/arch/x86_64/syscall.rs`, and this note. The scheduler and VFS
instrumentation is limited to the disposable guest configuration.
