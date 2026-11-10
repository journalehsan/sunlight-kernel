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

Host logic, native C/C++ header and symbol checks (no guest execution):

```sh
python3 tools/native-c/tests/run_unistd.py --out target/native-c-unistd-check
```

Two pre-existing secret-store tests issue native credential syscalls and are
excluded from this Linux-host harness. Native syscall numbers must never be
interpreted as Linux libc behavior.
