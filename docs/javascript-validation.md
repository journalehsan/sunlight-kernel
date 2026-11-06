# SunJS milestone validation

Validation performed on 2026-11-06 with the repository's pinned
`rustc 1.98.0-nightly (8954863c8 2026-06-05)` and Node v26.12.1.
This is targeted subset validation, not full ECMAScript conformance.
No Test262 cases were imported or executed.

| Check | Actual result |
| --- | --- |
| `cargo test -p sunlight-js -p sunlight-js-cli --target x86_64-unknown-linux-gnu` | PASS: 23 integration test groups (17 original plus six intrinsic/descriptor regressions), including source diagnostics, TDZ/const, closure/recursion, per-iteration bindings, this, evaluation order, objects/arrays/prototypes, native accessors/errors/reentry/rooting, limits, unsupported syntax and 200 teardown cycles |
| `cargo build -p sunlight-js-cli --target x86_64-unknown-linux-gnu` | PASS |
| Host `sunjs --smoke` | PASS |
| `python3 tools/test_sunjs_compare.py` | PASS: 82 selected Script completion comparisons with Node/V8, plus all six runnable examples |
| `cargo clippy -p sunlight-js -p sunlight-js-cli --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` | PASS |
| `cargo fmt -p sunlight-js -p sunlight-js-cli --check` and `git diff --check` | PASS |
| `cargo test -p golden-fish -p rappid-rabbit --lib --target x86_64-unknown-linux-gnu` | PASS: 26 Golden Fish and 170 Rabbit tests |
| `cargo test -p sunlight-ui --lib --target x86_64-unknown-linux-gnu` | PASS: 284 shared UI tests |
| `cargo test -p sunshell --lib --target x86_64-unknown-linux-gnu --no-default-features` | PASS: 28 shell library tests |
| `bash tools/build_sunjs.sh` | PASS: release native SunlightOS ELF through pinned std overlay |
| `bash tools/test_sunjs.sh` | PASS: native CLI and supporting service builds, kernel build, isolated Limine ISO, QEMU/KVM with two CPUs, real TTY login/foreground execution, smoke verdict and exit 0 |
| Python compilation and `bash -n` for added scripts | PASS |

The guest serial verdict is:

```text
[TTY]  cmd: /bin/sunjs --smoke -> [SUNJS] native smoke PASS\n
[TTY]  exit: /bin/sunjs --smoke -> 0
```

The guest also reports process reaping and resource cleanup. Full logs are local,
ignored artifacts under `target/`: `sunjs-host-tests.log`, `sunjs-host-build.log`,
`sunjs-clippy.log`, `sunjs-compare.log`, `sunjs-browser-regression.log`,
`sunjs-ui-regression.log`, `sunjs-shell-regression.log`,
`sunjs-final-guest-gate.log`, and `sunjs-guest.log`.

The guest workflow exposed two infrastructure requirements: each supporting
service must be built separately to avoid unifying conflicting sunlight-libc
allocator features, and the shell must recognize `/bin/sunjs` as a kernel
embedded executable because it is not exposed by VFS stat. The new gate follows
the existing QMP login pattern and waits for shell registration before sending
the command; a fixed-time injected sequence did not produce a CLI verdict.

The workspace still emits existing duplicate-bin, libc, UI, and network warnings
during some builds. The new engine/CLI host Clippy gate is clean. Whole-workspace checks were attempted for this object-model milestone:
`cargo test --workspace --target x86_64-unknown-linux-gnu` fails compiling
`wiseowl-brain` (`std` unavailable in error.rs and missing `String` imports in
protocol.rs); `cargo check --workspace` fails because std-dependent crates such
as `float_cmp` and `log` cannot build for the default `x86_64-unknown-none` target.
These failures are outside SunJS. The broad `tools/test.sh` phase matrix was not
run. This workspace mixes freestanding kernel/services, native std and host targets. The relevant
browser/document/UI/shell host suites and the new full native boot path were run.
No browser script execution, live-site checks, full standard library tests,
heap/RSS profiling, tracing-GC stress testing, or full ECMAScript conformance is
claimed. Arena retention and supported syntax restrictions are documented in
[javascript.md](javascript.md).


The object-model regressions cover intrinsic constructor/prototype identity,
multilevel shadowing, inherited JavaScript and Rust accessors with receiver
identity, descriptor defaults/redefinition/SameValue, sparse slice, readonly
length, non-configurable truncation barriers, reentrant length coercion, and
call/apply with strict receivers. Legacy grammar rejection and all 200 existing
teardown cycles still pass. The object/descriptor changes add no dependencies
and do not enable Rabbit script execution.
