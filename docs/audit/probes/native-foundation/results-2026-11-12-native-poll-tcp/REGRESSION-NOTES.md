# Regression notes

- `cargo test -p sunlight-libc --target x86_64-unknown-linux-gnu --features native-c`
  was stopped after more than four minutes. The run was blocked in
  `secret_store::tests::default_options_are_private_and_bounded` and
  `secret_store::tests::path_policy_rejects_traversal_and_staging_names`.
  A clean `HEAD` worktree reproduces the same timeout (`baseline-head-secret-store-timeout.log`,
  exit 124).
- Re-running with `--skip secret_store` gave 111 passed and 7 failed
  (`regress-libc-test-skip-secret-store.log`). The seven failures are in
  `alloc::tests::*` and `dirent::tests::empty_streams_allocation_failure_and_close`.
  The same seven fail in a clean `HEAD` worktree
  (`baseline-head-alloc-dirent-tests.log`). They are pre-existing host-test
  failures, not caused by the poll/TCP change.
- `cargo check -p sunlight-kernel --target x86_64-unknown-none` passed.
- `cargo check -p sunlight-net-server -p sunlight-ipc --target x86_64-unknown-none`
  passed (existing mutable-static warnings only).
