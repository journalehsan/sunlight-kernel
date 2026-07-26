# Phase 2B.1 — Foundational process and pathname utilities

This ledger records the POSIX.1-2024 design target for the first foundational
`sunlight-utils` expansion. It is an implementation record, not a claim of
POSIX or UNIX certification.

## Pre-edit repository trace

| Command | Existing form | Current runtime | Required APIs | Installed mapping | Action |
|---|---|---|---|---|---|
| `true` | No standalone binary, shell built-in, archived proof, incomplete module, test, or old image mapping found | None | libc startup, bounded argv collection, `exit` | None | Add standalone libc binary and base-image mappings |
| `false` | No standalone binary, shell built-in, archived proof, incomplete module, test, or old image mapping found | None | libc startup, bounded argv collection, `exit` | None | Add standalone libc binary and base-image mappings |
| `basename` | No standalone binary or utility module found; only unrelated lexical helpers and documentation references | None | libc startup, bounded byte argv, stdout/stderr, `write_all`, `exit` | None | Add standalone libc binary, shared lexical helper, and base-image mappings |
| `dirname` | No standalone binary or utility module found; only unrelated lexical helpers and documentation references | None | libc startup, bounded byte argv, stdout/stderr, `write_all`, `exit` | None | Add standalone libc binary, shared lexical helper, and base-image mappings |
| `echo` | Standalone migrated binary plus legacy multicall applet | `sunlight-libc` `_start` → `collect_utf8_args` → `write_all` → `exit` for installed `/bin` and `/usr/bin` forms | startup, argv, stdout, exit | `/bin/echo`, `/usr/bin/echo`, `/sunlight-utils/echo` (the latter via kernel resolver) | Regression reference; unchanged |
| `pwd` | Standalone migrated binary plus legacy multicall applet | `sunlight-libc` `_start` → raw argv → `getcwd`/`write_all` → `exit` | startup, argv, cwd, stdout/stderr, exit | `/bin/pwd`, `/usr/bin/pwd`, `/sunlight-utils/pwd` | Regression reference; unchanged |
| `cat` | Standalone migrated binary plus legacy multicall applet | `sunlight-libc` `_start` → raw argv → `open/read/close/write_all` → `exit` | startup, argv, file I/O, stdout/stderr, exit | `/bin/cat`, `/usr/bin/cat`, `/sunlight-utils/cat` | Regression reference; unchanged |
| `z` | Separate `sunlight-zoxide` binary | Existing native userland runtime for that package | Package-specific; outside this phase | `/bin/z`, `/usr/bin/z`, `/usr/local/bin/z` | Regression reference; unchanged |

The package currently contains one legacy multicall binary (`sunlight-utils`)
and standalone migrated binaries for `echo`, `cat`, and `pwd`. The kernel build
script builds the package for `x86_64-unknown-none` with the user-space linker
flags and embeds the resulting ELFs. `/bin` stubs and `/sunlight-utils` stubs
provide normal shell lookup; the kernel's embedded-path resolver selects the
standalone ELF for migrated commands. There is no `/usr/bin` initramfs stub
directory, so this phase follows the existing base-image policy rather than
adding one.

The maintained libc path provides the native `_start` ABI (`argc` in `rdi`,
`argv` in `rsi`, `envp` in `rdx`), bounded raw/UTF-8 argv helpers, `write_all`,
`exit`, panic handlers in each native utility, and no allocator for these
fixed-buffer commands. The pathname commands use raw bounded argv slices so
their output is byte-preserving; Sunlight's documented process-entry contract
currently guarantees UTF-8 argv, so arbitrary non-UTF-8 pathname execution is
not a separately promised Sunlight interface.

The existing acceptance area is `tools/tests`: its `.expected` files contain
serial substrings, while `tools/test.sh` builds the default test image and can
enable the existing `key_inject` feature. `phase6.5.utils` is the maintained
native utility and directory-lifecycle regression reference. No standalone
parallel runner or expectation format is introduced by this phase.

## Specification references

The design target is the Open Group Base Specifications Issue 8, IEEE Std
1003.1-2024 (POSIX.1-2024), using these normative pages:

- XCU `true`: <https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/true.html>
- XCU `false`: <https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/false.html>
- XCU `basename`: <https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/basename.html>
- XCU `dirname`: <https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/dirname.html>
- XBD 12.2, Utility Syntax Guidelines: <https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/basedefs/V1_chap12.html>

The basename and dirname transformations are implemented from the ordered
steps in their XCU DESCRIPTION sections. The public POSIX manual mirror was
also consulted for the same ordered text while the official utility pages were
accessed by their Issue 8 URLs.

## Conformance ledger

| Utility | Target specification | Required forms | Implemented | Tested | Known deviations |
|---|---|---|---|---|---|
| `true` | POSIX.1-2024 XCU `true` DESCRIPTION/EXIT STATUS | `true` | Standalone libc utility; no output; status 0; extra arguments are ignored as a documented Sunlight extension | Host table tests; target ELF; real-target status not verified | POSIX defines no operands and does not specify non-conforming extra-operand behavior; Sunlight accepts them |
| `false` | POSIX.1-2024 XCU `false` DESCRIPTION/EXIT STATUS | `false` | Standalone libc utility; no output; deterministic status 1 | Host table tests; target ELF; real-target status not verified | POSIX requires non-zero, not a particular number; Sunlight selects and documents 1 |
| `basename` | POSIX.1-2024 XCU `basename` DESCRIPTION/STDOUT/EXIT STATUS | `basename string [suffix]`; `--` delimiter accepted before operands | Ordered lexical transformation; byte-preserving output; one newline; status 0; usage status 2 with deterministic stderr | Table-driven host tests; target ELF; real-target output observed in an earlier focused boot, final full case not verified | POSIX pathname values are byte-oriented while current Sunlight process argv is guaranteed UTF-8; max argument length is the maintained 255-byte payload bound. Empty string returns empty and repeated `//` is processed to `/`, both permitted choices |
| `dirname` | POSIX.1-2024 XCU `dirname` DESCRIPTION/STDOUT/EXIT STATUS | `dirname string`; `--` delimiter accepted before the operand | Ordered lexical transformation; byte-preserving output; one newline; status 0; usage status 2 with deterministic stderr | Table-driven host tests; target ELF; real-target output observed in an earlier focused boot, final full case not verified | POSIX pathname values are byte-oriented while current Sunlight process argv is guaranteed UTF-8; max argument length is the maintained 255-byte payload bound. Repeated `//` is processed to `/`, the permitted implementation-defined choice |

## Target behavior choices

`true` and `false` do not parse options, access the filesystem, consult the
environment, allocate, or wait. `false` returns status 1 because POSIX only
requires a non-zero result; the numeric choice is fixed so Sunlight behavior is
deterministic and testable.

`basename` removes trailing slashes, returns `/` for an all-slash operand,
removes the prefix through the final remaining slash, and removes a suffix
only when it is a non-identical suffix of the resulting basename. `dirname`
removes trailing slashes, returns `.` when no slash remains, removes the final
non-slash component, then removes trailing slashes and returns `/` if that
leaves an empty result. Both utilities process `//` using the all-slash path,
thereby choosing `/` rather than preserving `//`.

All normal pathname output is written with libc `write_all` and ends with one
newline. Invalid operand counts produce no stdout, a short utility-specific
usage diagnostic on stderr, and status 2. No GNU-only options are provided.

## Validation evidence

`cargo test -p sunlight-utils --target x86_64-unknown-linux-gnu --lib` passed
19 tests, including exact stdout/stderr/status tables. Debug target symbols show
`_start`, `sunlight_libc::crt0::collect_raw_args`, `write_all`, and `exit` in
all four ELFs; the pathname ELFs also contain only the shared lexical helpers,
and none contains direct filesystem or IPC symbols. A focused boot with the
existing image/injection path produced the exact pathname results
`kernel\\n`, `/root/projects/sunlight\\n`, `name\\n`, and `/tmp/path\\n`, each with
status 0 and process cleanup records showing `ipc_entries_cleared=0` and
`owned_frames=0`. The same run did not reach the no-output `true`/`false`
diagnostic lines. A later final run reached the injector completion marker
before shell readiness and is therefore not counted as a complete acceptance.

## Certification gaps and unverified cases

This phase does not claim complete POSIX conformance. Full arbitrary-byte argv
execution, locale-sensitive behavior, pathname operands containing a newline,
resource-growth measurement over a long soak, all `/usr/bin` lookup forms, and
the complete four-command keyboard acceptance sequence are not verified. The
existing injector reached its completion marker before shell-session readiness
in the final run, so no new shell or keyboard architecture was added to mask
that harness race.
