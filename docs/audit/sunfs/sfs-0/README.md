# SFS-0 design and audit evidence

2026-11-14. Entry point: `validation-summary.json` for gate status and evidence boundary.

- `repository-capability-inventory.md`: source owners, reusable contracts, limitations,
  dependencies, PY-5/6/7 history and current-source corrections.
- `existing-draft-comparison.md`: v0.1 retained/refined/deferred decisions.
- `block-device-contract-assessment.md`: existing versus required persistence semantics.
- `architecture-decision-summary.md`: selected architecture and alternatives.
- `format-field-inventory.json`: machine-readable proposed widths/offset accounting;
  accompanies the normative format document, not a production codec.
- `geometry-model.json`: arithmetic only, not a workload benchmark or final geometry.
- `recovery-invariants.md`, `risk-register.md`: hazards, prevention, future proof gates.
- `source-baseline.json`: incoming tracked/unignored regular-file hashes and git state.
- `source-integrity-manifest.json`: verification that baseline bytes remain unchanged,
  output hashes and explicit excluded scope. Excludes its own hash to avoid circularity; the validation summary is included
  among its output hashes.
- `source-anchors.json`: reviewed source paths, anchor line numbers and content hashes.
- `validate.py`, `validation-checks.json`: reproducible document/field arithmetic/link/
  CRC-vector/source-integrity checks. These do not serialize a SunFS image or exercise
  any recovery engine, driver, filesystem, guest or UI.

All production/runtime/crash/serialization tests are NOT_RUN in this milestone.
No historical PASS result is promoted to a new execution. Design gates can PASS while
implementation and physical durability remain unproven. No commit or push was made.
