# SunlightOS Boot Gate Tests

This directory holds deterministic serial-output expectations and future test
injection notes for `tools/test.sh`.

Each `*.expected` file contains one required serial substring per line. The main
test runner should treat blank lines and `#` comments as non-matching metadata.

Planned runner entry points:

```bash
./tools/test.sh phase2.6
./tools/test.sh phase3.0
./tools/test.sh phase3.5
./tools/test.sh phase3.6
./tools/test.sh audio
```

The default `./tools/test.sh` should run the latest stable gate. Keep expected
strings short, deterministic, and tied to behavior that must not regress.

Phase-specific implementation notes belong in:

```text
docs/PHASE_3_0_SUMMARY.md
docs/PHASE_3_5_SUMMARY.md
docs/PHASE_3_6_SUMMARY.md
```

Fast host-side policy checks can live alongside the boot-gate data when they
validate launcher-only behavior. Current example:

```bash
./tools/tests/test_vm_display_policy.sh
```
