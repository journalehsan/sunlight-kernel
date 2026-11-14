# Historical native cpp-runtime link failure — investigation and closure

Scope: the historical native suite row `cpp-runtime: LINK_FAIL, runtime
NOT_RUN`. This milestone does not touch TCP LT epoll networking semantics.

## 1. Starting state

- HEAD `5b08147001be963e468711d03284b349fa8b2089` (see `00-starting-state.txt`).
- Worktree carries incoming uncommitted TCP LT epoll contract-closure work and
  untracked evidence; all preserved, nothing committed, cleaned, reset or
  switched.
- Toolchain and target: see `01-toolchain.txt` (clang/lld, `x86_64-sunlightos`).
- Native archives (before): `02-archive-hashes-before.txt`.
  `libsunlight_native_c.a = f1fe24486aa0152688408c16cb3ddddb793ef1024d582111f92cd33da4787441`
  (matches the reported archive), plus `libsunlight_cxx.a` and `libsunlight_eh.a`.
- Exact original cpp-runtime cell command (bare recipe):
  `clang++ --target=x86_64-unknown-none-elf -nostdinc -I<libc>/include -nostdlib -fuse-ld=lld docs/audit/probes/cpp-runtime.cpp -o cpp-runtime.elf`
  (`03-baseline-historical-link.log`, `04-baseline-reproduction.log`).
- Current full linker diagnostic: **20 unresolved C++ symbols** emitted by the
  bare recipe (exception, RTTI, `std::string`, iostream, guard/EH, operator new,
  `atexit`). Reproduced unchanged under a watchdog (`04`).

### The reported `close` symptom belongs to a different probe

`undef close` is the historical **`socket.c`** bare-command failure, not
cpp-runtime (`06-historical-socket-close.log`,
`results-2026-11-12-native-net/historical/original-command.log`). The
cpp-runtime fixture never references `close`.

## 2. Root-cause trace

Category **A. Build/link recipe defect** — not B, C or D.

- `cpp-runtime.o` undefined set (`05-symbol-trace.txt`, `13-provenance.txt`):
  `_Unwind_Resume`, `__cxa_*`, `__gxx_personality_v0`, `_ZTISt13runtime_error`,
  `_ZNSt6string*`, `_ZSt4cout`, `_Znwm`/`_ZdlPvm`, `atexit`, `pthread_create`,
  `pthread_join`, `__dso_handle`, … **`close` is absent.**
- `close` is a strong native export: `T close` in the target-built
  `libsunlight_native_c.a`. The C++ archives have no `close` reference. There is
  no missing or incorrectly gated libc export (B ruled out).
- The bare recipe passes no native startup, no libc archive, no C++/EH archives
  and no linker script, so nothing can satisfy the C++ runtime references. It is
  the recipe, not the runtime integration (C ruled out) or unsupported
  functionality (D ruled out). All 20 symbols are defined by the configured
  driver's archives.
- C fixtures link `close` because they use `cc.py`, which passes
  `libsunlight_native_c.a`, `-nostdlib -static` and `user-space.ld`. Only the
  historical C++ cell used the bare command. `cxx.py` already composes
  `cc.py` plus the C++/EH archives and `--wrap=main`.

## 3. Minimal fix

`tools/native-c/guest.py`: route every C++ cell (including historical probe
sources) through the configured `tools/native-c/cxx.py` driver — the same driver
every other native cell uses. Removed the special-case that built the historical
C++ cell with the bare `clang++ -nostdlib` recipe.

`tools/native-c/tests/run_cxx_runtime.py`: kept the bare command as an explicit
LINK_FAIL regression and added focused assertions proving the real root cause —
`close` is a strong native export, `cpp-runtime.o` never references `close`,
it does reference `__cxa_atexit`/`__gxx_personality_v0`, and the linked ELF map
includes all three intended native archives.

No `-nostdlib` removal, no fixture change, no symbol suppression, no success
stub, no host libc, no hosted build, no networking change.

## 4. Build and provenance

- Ordinary locked native-C build (`11-build-native-c-locked.log`): PASS.
- Fresh archive hash identical to the reported value (`12-archive-hashes-final.txt`):
  `libsunlight_native_c.a = f1fe24…7441`, `libsunlight_cxx.a = 032bce…77d1`,
  `libsunlight_eh.a = cc703d…b512`. Only the link recipe changed; no source
  change altered the archives.
- New cpp-runtime cell command (`suite-2/build-cpp-runtime.log`):
  `python3 tools/native-c/cxx.py -Wall -Wextra -Werror docs/audit/probes/cpp-runtime.cpp -o cpp-runtime.elf` → exit 0.
- Linked ELF: `EXEC`, no `INTERP`, no `NEEDED`, no `DYNAMIC`; `.eh_frame`,
  `.eh_frame_hdr`, `.gcc_except_table`, `.tbss` present; zero undefined symbols;
  map references the three `target/native-c/sysroot/lib/libsunlight_*.a` archives
  (`13-provenance.txt`, `07-run-cxx-runtime-link-check.log`).
- Fixture integrity: `cpp-runtime.cpp` sha256 unchanged vs `HEAD`
  (`6de2fb45…19e3`).

## 5. Fixture execution

| Config | cpp-runtime link | cpp-runtime runtime | assertions |
| --- | --- | --- | --- |
| 2 CPUs | PASS | PASS exit 0 | `exception OK`, `TLS OK`, `static initialization OK`, `atexit OK`; reap `owned_frames=0 fds=0` |
| 4 CPUs | PASS | PASS exit 0 | identical stdout, identical reap |

Runtime PASS is reported separately from LINK_PASS. The unchanged fixture
exercises exception unwinding, RTTI `dynamic_cast`/`typeid`, `std::string`
static init, thread-local `std::string` in a `std::thread`, and `atexit`. Local
static guard ABI coverage remains in `run_cxx_runtime.py` (`__cxa_guard_*`,
`__sl_cxx_guard_thread_id`), unchanged.

## 6. Regressions

| Gate | Result |
| --- | --- |
| Full native suite, 2 CPUs | **94 rows, all PASS** (`Historical 21: PASS=21`), 0 LINK_FAIL/NOT_RUN |
| Full native suite, 4 CPUs | **94 rows, all PASS**, 0 LINK_FAIL/NOT_RUN |
| `run_cxx_runtime.py` (link/ABI/EH/guard) | PASS (`07-…log`) |
| `cargo check -p sunlight-kernel --target x86_64-unknown-none --no-default-features` | PASS (`14-kernel-check.log`) |
| `tools/build_native_c.sh` (`--locked`) | PASS (`11-build-native-c-locked.log`) |
| `git diff --check` | clean (`09-diff-check.log`) |

The TCP LT epoll networking acceptance matrix was not rerun: changes are
isolated to the native C++ build/selection recipe and C++ link regression,
and no shared libc behavior changed (archives byte-identical).

## 7. Remaining limitations

- The historical bare `clang++ -nostdlib` command remains LINK_FAIL by design;
  it is preserved and asserted as such, never reclassified as a driver defect.
- All prior LINK_FAIL reports are unchanged; these PASS results are new evidence.

## 8. Repository status

No commit. Incoming tracked/untracked work and archives preserved. Changed
files: `tools/native-c/guest.py`, `tools/native-c/tests/run_cxx_runtime.py`.
