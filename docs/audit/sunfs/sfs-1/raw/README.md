# Raw evidence provenance

The final command authority is ../commands.json. Its compiled_source_sha256 binds
the Rust implementation, fixtures and runner used in the final two-profile campaign.
Final tests are tests-dev-final.log and tests-release-final.log; final corpora,
crash traces and deterministic compressed fixture images are in dev/ and release/.
Tests overwrite only those explicitly regenerated final outputs on a new campaign.

Earlier logs are retained development attempts, not validation of the final tree:

- core-check-initial.log: manifest discovery before all crate targets existed.
- core-tests-initial.log: initial core codec/geometry suite, 10 tests.
- lab-tests-initial.log: 11 pass, 1 invalid bitmap test-fixture failure.
- lab-tests.log and all-tests.log: intermediate passing source snapshots.
- nonempty-checker-initial.log: initial nonempty semantic fixture validation.
- clippy-initial.log/clippy-second.log: initial lint findings and correction.
- performance-dev.jsonl and performance-dev-build.log: intermediate diagnostics.
- source-integrity-first-attempt.json and validation-run.log: final new-file scan
  accidentally included pre-existing symlinks excluded from the regular baseline;
  all 5434 unexpected entries were symlinks, with no changed/missing regular files.
- strict-fault-oracle.log, strict-oracle-all-tests-failure.log and
  validation-run-final.log: overly strict oracle assumed a CP must persist all4096
  bytes to recover. A511-byte prefix contained every nonzero byte; untouched fresh
  slot tail zeros made a complete page. Corrected by comparing full durable bytes.
- strict-fault-oracle-corrected.log: targeted 480-case rerun passed before the final
  two-profile campaign.
- segment-state-reproduction.log: actual codec validation defect, RETIRED with live
  blocks accepted. Corrected to require no RETIRED current ownership and nonzero
  epochs in every non-FREE state; final tests include the two regressions.
- root fuzz-corpus.jsonl: intermediate campaign; use dev/ and release/ for final
  classifications and reproducer identities.

validation-run-completion.log records the final full runner invocation. A last
integrity-only invocation runs after this log closes, because an active redirected
log cannot have a stable final digest while its process is still appending to it.
No earlier PASS is substituted for final-source validation. Native guest/physical
storage durability remains unqualified throughout.
