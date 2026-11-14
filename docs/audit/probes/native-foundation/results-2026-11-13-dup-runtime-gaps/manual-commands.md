# Commands before the recorded runner

Working directory: repository root. D below abbreviates exactly:
`docs/audit/probes/native-foundation/results-2026-11-13-dup-runtime-gaps`.
Acceptance commands with full argv and watchdog exits are in
`validated/commands.json`, `run-2/commands.json` and their guest logs.

Initial production rebuild (exit 0, build-native.log):

```sh
timeout 240 bash tools/build_native_c.sh
```

Adapter attempts used this command with the changes listed below:

```sh
timeout 60 target/native-std/rustc --edition=2021 --crate-type=lib --emit=obj -C opt-level=0 -C panic=abort --target tools/targets/x86_64-sunlightos.json -Z json-target-spec -L dependency=target/native-c/x86_64-sunlightos/release/deps --extern sunlight_libc=target/native-c/x86_64-sunlightos/release/deps/libsunlight_libc-2d48049d6dc48c05.rlib tools/native-c/tests/dup2-wrapper-adapter.rs -o "$D/adapter.o"
```

- adapter-build.log: exact command above; BUILD_FAIL.
- adapter-build-2.log: omit `-Z json-target-spec`; BUILD_FAIL.
- adapter-build-3.log: replace that flag with `-Z unstable-options`; BUILD_FAIL.
- adapter-build-4.log: also add `--extern core=target/native-c/x86_64-sunlightos/release/deps/libcore-98dfb8ed3b8bdfbd.rlib`; PASS.

Each was followed by the same link command (wrapper-link.log,
wrapper-link-2.log, -3.log, -4.log). After fixing the declaration,
wrapper-link-5.log uses that exact command and passes:

```sh
timeout 60 python3 tools/native-c/cc.py -Wall -Wextra -Werror -DWRAPPER_TEST tools/native-c/tests/dup-runtime.c "$D/adapter.o" -Wl,-Map="$D/wrapper.map" -o "$D/wrapper.elf"
```

The initial Linux handler link passed:

```sh
timeout 60 python3 tools/native-c/cc.py -Wall -Wextra -Werror tools/native-c/tests/dup-runtime.c -Wl,-Map="$D/linux-handler.map" -o "$D/linux-handler.elf"
```

Initial wrapper execution (wrapper-2.log, runtime FAIL on two invalid
immediate-counter assertions; full output/ISO/kernel preserved):

```sh
timeout 240 python3 tools/native-c/guest.py "$D/wrapper.elf" --net --cpus 2 --timeout 100 --expect-exit 0 --out "$D/wrapper-2"
```

Orchestrators, all command-level watchdogs recorded by the script:

```sh
python3 tools/native-c/tests/run_dup_runtime.py --out "$D/final" --suite
python3 tools/native-c/tests/run_dup_runtime.py --out "$D/run-2" --suite
python3 tools/native-c/tests/run_dup_runtime.py --out "$D/focused-final" --focused-only
python3 tools/native-c/tests/run_dup_runtime.py --out "$D/validated" --focused-only
```

The first stops at Linux-actor LINK_FAIL; run-2 retains superseded focused
failures but all adjacent regressions and suites pass. focused-final retains
the cross-round counter-oracle failures. validated is final focused PASS.
The orchestrator was subsequently changed to exit nonzero if any recorded
runtime result fails; older process exit 0 never served as acceptance proof.

```sh
timeout --kill-after=10 120 python3 "$D/summarize.py"
```

Result: PASS. Final formatting, syntax, ABI-negative command and diff checks
have explicit separate logs. Initial read-only failed path lookups and the
no-op failed patch are described in REPORT.md; they produced no artifacts
or production mutations.
