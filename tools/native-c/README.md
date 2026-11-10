# Native C guest acceptance

Build the canonical runtime, compile with the native driver, then run in a new
headless VM (from the repository root):

```sh
bash tools/build_native_c.sh
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror \
  docs/audit/probes/native-foundation/entry.c -o target/native-c-entry
python3 tools/native-c/guest.py target/native-c-entry \
  --out target/native-c-entry-run
```

An explicit evidence directory must be new; without `--out`, the runner chooses
a unique directory under `target/native-c-guest-runs/`. The runner never executes a native ELF on
Linux. Support and kernel builds use dedicated caches under
`target/native-c-guest/`, and each run snapshots its own kernel ELF before
creating its ISO. It creates its own ISO staging tree, attaches only that read-only CD,
disables networking, uses TCG and virtio-rng, and kills QEMU at the 90-second
guest deadline. `--cpus 1`, `2` (default), or `4` selects the guest CPU count.
Build logs, exact QEMU argv, raw serial, decoded stdout/stderr, ELF hashes and
results are retained. Generated binaries and media are ignored in audit folders.

Single-ELF mode returns the observed guest exit code. To check a deliberate
nonzero exit as a successful assertion, supply `--expect-exit 23`. Infrastructure,
timeout, exception, unexpected syscall, output-length or cleanup failures return
nonzero even if the guest requested exit zero. Arguments follow repeatable
`--arg VALUE`; input bytes come from `--stdin-file PATH` (otherwise stdin is an
empty file). Standard streams are separate ordinary tmpfs files, so they support
EOF and seeking; this fixture does not exercise terminal or pipe behavior.
The captured bytes are encoded into bounded hexadecimal serial records and
reconstructed exactly, including binary bytes. Each capture may contain at most
1 MiB when collected; larger captures fail the fixture.

Run the complete acceptance suite:

```sh
python3 tools/native-c/guest.py --suite \
  --out docs/audit/probes/native-foundation/results-NEW-guest
```

The suite rebuilds all 21 historical executable cases plus stdout, full stdio,
raw open/unlink, and deliberately unclosed tmpfile fixtures. Phase 5 adds nine
separate write/termination cases from `tests/unistd.c`: basic, zero count, short
write/stdio EOF, write errors, callback order, callback registration, registry
capacity, explicit C exit and raw runtime exit. These do not inflate historical
API counts. Known missing-header build failures are
`acceptance=BUILD_FAIL, guest_run=NOT_RUN` with a build log; they are
never described as observed guest runtime failures. The suite exits zero only
when every buildable case passes its exit/output and reaper checks. `compiler.c`
is compile-only and is not one of the 21 executable cases.

The opt-in `native_c_guest` kernel feature reads the generated manifest through
`SUNLIGHT_NATIVE_C_MANIFEST`. It excludes desktop service payloads and boot
self-tests, mounts a minimal
root with the canonical mode-1777 `/tmp`, starts the actual init nameserver
with spawn token zero and actual `rand_service`, waits for successful random
service registration and receive wait, places each ELF
at `/tmp/native-c.elf`, reads it back, and uses the ordinary `exec_into_process`
loader with native stack/argv/envp/auxv and TLS startup. Tests run sequentially as
uid/gid 1000, with no privileged service roles. `/tmp/native-c-work` is the
suite's disposable argv fixture and is removed between cases. The ELF header,
program-header, address-range and W^X checks are unchanged. Unsupported or
malformed images still fail normal loading. The fixture limits ELF file bytes
to 64 MiB; this is a runner resource bound, not a new kernel ABI limit.

Completion waits for the actual scheduler `Reaped` state, matching the exit
record with reclamation stats, zero PID-owned frames, empty descriptors and a
released kernel stack. After capture/fixture deletion, tmpfs file-data frames,
user-private frames, page-table frames, VFS open handles and visible `/tmp`
entries must equal the pre-launch snapshots. The exit-cleanup fixture leaves
both anonymous files (three data pages) and heap allocations open deliberately.
These checks exercise normal process exit, not forced crash/thread races.

The API audit's `NATIVE_RUNNER` hook can also use single-ELF mode:

```sh
NATIVE_RUNNER='python3 tools/native-c/guest.py' \
  python3 docs/audit/probes/run.py --targets native \
  --out docs/audit/probes/results-NEW
```

Each invocation automatically retains its own evidence directory.
`tools/native-c/tests/run_stdio.py` remains a host-adapter regression runner and
does not itself run the guest acceptance gate.

The runner also rejects the kernel's actual `[FAULT]` records even when an
expected status matches. The negative `tests/fault.c` control uses `ud2`; its
expected native termination status is 132, while the host runner must fail.
This negative control is separate from the positive suite cases.

The native C package enables libc's `native-c` feature, which adds C `write`,
`atexit` and `exit`. C main return runs callbacks in reverse order before the
canonical group exit; the registry holds up to 32 at once. Callbacks may register more
callbacks as slots become free. `sl_std_exit` and Rust-only startup retain their
raw behavior. `unistd.h` currently declares only `write`, the native LP64 count
types and standard descriptor constants; it advertises no POSIX version.
Zero-count writes do not validate nonnegative descriptor existence, and generic
native user-copy errors still map to EIO. No terminal, signal or full POSIX
semantics are implied by file-backed guest acceptance.

Phase 6 adds `<sys/random.h>` and the C `getrandom(void *, size_t, unsigned int)`
export. Only flags zero is supported; all other bits fail EINVAL, including the
Rust-only bit 1 that selects libc's local PRNG. Nonempty calls use the production
`rand` capability-IPC service, with 32-byte replies and no fallback. Service
failures map to EIO, NULL/nonzero to EFAULT, oversized counts to EINVAL; success
preserves errno. Zero length never accesses the buffer or the service. Nonempty
non-NULL buffers must be writable; arbitrary mapping/protection faults cannot
be recovered by this userspace adapter.

The suite brackets unchanged historical `getrandom.c` with two independent
STATS observers. `random-service.log` requires an exact increase of one request
and 16 bytes, proving service use for that process. Four separate random
contracts cover basic requests, zero length, invalid arguments and chunk/guard
boundaries; each checks exact service counter deltas. The observers expose only
counts, never random bytes or capability tokens. `tests/run_random.py --out NEW`
runs production libc host unit tests and twelve C11/C++17 ABI/header checks;
it builds the original probe but never executes native ELFs on Linux.

Phase 7 adds the narrow `<time.h>` surface: signed LP64 `time_t`/`clock_t`,
32-bit signed `clockid_t`, a 16-byte `struct timespec`, `CLOCK_REALTIME=0`,
`CLOCK_MONOTONIC=1`, `clock_gettime` and relative `nanosleep`. Declarations are
available with base, POSIX and GNU feature macros without advertising POSIX
conformance. Realtime is native UTC epoch seconds with zero nanoseconds;
monotonic time uses the canonical kernel 100 Hz timekeeper. Neither exposes
sub-tick precision or a wall-clock accuracy guarantee.

Sleep uses the same native futex transport as Rust std, with a checked absolute
monotonic deadline and clock rechecks after wakes. Negative seconds, invalid
nanoseconds, duration/deadline overflow and the indefinite-wait sentinel fail
EINVAL; NULL request/output pointers fail EFAULT. Non-NULL input/output storage
must be valid; arbitrary mapping faults are not recovered by these userspace
adapters. Native waits have no signal-interruption/EINTR contract. The remainder
is never accessed and stays unchanged on completed sleeps and failures. Zero
sleep does not access a clock or wait. Success preserves errno.

The suite now runs **31 built executions**: unchanged historical `clock.c` plus
four separate clock/error/zero/sleep contracts join the previous 26 cases. The
sleep fixture checks 1 ns, 1 ms, 25 ms and 1 s + 1 ns requests against monotonic
time, allowing up to 5 seconds of scheduling lateness. Captured durations are
in the native coarse clock domain; they do not measure physical timing accuracy.
`tests/run_time.py --out NEW` runs production libc tests, twelve C/C++ layout,
type/signature/include checks, canonical-export uniqueness, and strict native
ELF validation of the unchanged original source. It never runs a native ELF
on Linux.

Phase 8 audits the existing mapping ABI. `<sys/mman.h>` supplies native LP64
`off_t`, constants and declarations; the existing Rust mmap/munmap validation,
syscalls and C exports remain intact. The new C `mprotect` adapter invokes
existing native syscall 52, which updates real PTE permissions and performs TLB
invalidation after full-range preflight. Native failures currently map to EIO;
local request validation maps to EINVAL. `MAP_NATIVE_THREAD` stays internal.

The unchanged historical `mmap.c` now builds and exits 0 on **2 and 4 CPUs**.
Both full suites pass **35 built executions** (historical **9 PASS / 12
BUILD_FAIL**, no guest failures). Three separate mapping fixtures verify the
exact 1 MiB anonymous/private RW subset, zero fill, RW-to-R-to-RW changes using
independent kernel writable-PTE checks, middle-page MAP_FIXED replacement with
neighbor preservation, and overflow/user-boundary rejection without mutation.
The fixed test establishes that case; it does not cover every replacement or
failure interleaving. Hardware user-write fault and forced stale-TLB tests were
not run. `tests/run_mman.py --out NEW` runs libc host tests, twelve C/C++ ABI
checks, unique-export checks and strict native ELF validation of unchanged
`mmap.c`. Native executables run only through the guest runner.

Host logic, native C/C++ header and symbol checks (no guest execution):

```sh
python3 tools/native-c/tests/run_unistd.py --out target/native-c-unistd-check
```

Two pre-existing secret-store tests issue native credential syscalls and are
excluded from this Linux-host harness. Native syscall numbers must never be
interpreted as Linux libc behavior.

Phase 9 adds the narrow `<dirent.h>` bridge: opaque `DIR`, `opendir`, `readdir`,
`closedir`, and `struct dirent` containing only `d_name[65]`. It uses native
getcwd/stat/enumeration (64/61/148), with no Linux ABI or VFS service substitute.
Only `"."` and canonical absolute paths are supported; other pathname forms
fail ENOSYS. Each stream owns its path, offset and returned entry. Enumeration
uses real native records, preserves errno on success/EOF, inserts no synthetic
dot entries, and closes by freeing its userspace state. No directory descriptor,
inode/type fields, seek, stable snapshot, or concurrent mutation contract is
provided. Callers serialize each live stream; dangling pointers are invalid.

The unchanged historical `dirent.c` passes on 2 and 4 CPUs. Both final full
suites pass **39 built cases**; the historical matrix is **10 PASS / 11
BUILD_FAIL / 11 guest NOT_RUN**. Three focused directory cases cover independent
streams, EOF, errors and permission denial, 71 real entries including a 64-byte
name, and opening `"."` before a CWD change. The opt-in guest image supplies
fixed empty/writable and mode-000 directories before cleanup snapshots. Earlier
failed attempts that tried to remove directories using native unlink remain
retained; native directory removal is outside this phase. Run
`tests/run_dirent.py --out NEW` for 114 libc host tests (2 native credential tests
filtered), twelve C/C++ ABI checks and unchanged historical native ELF validation.
Guest acceptance remains separate. Phase 8 hardware write-fault and forced
stale-TLB tests remain NOT_RUN.
