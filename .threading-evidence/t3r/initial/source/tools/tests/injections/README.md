# Kernel Test Injection Notes

This folder is reserved for deterministic test-injection helpers that later
phases can enable from `tools/test.sh`.

Use injection only when normal QEMU automation is brittle or impossible. Keep
the injection path explicit in serial logs so test output proves what happened.

Planned examples:

- Phase 3.0: VFS service self-test requests for open/read/stat/error paths.
- Phase 3.5: deterministic block read checks against `target/test.img`.
- Phase 3.6: keyboard injection sequence:

```text
[KBD]  Injecting test keys: root<tab>root<enter>
```

Rules:

- Injection must be disabled unless a test mode or boot-gate path requests it.
- Injection must not replace the real implementation path.
- Every injected scenario needs an expected serial line and a failure serial
  line where practical.
