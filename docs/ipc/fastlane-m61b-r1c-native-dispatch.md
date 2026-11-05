# M6.1B-R1C authenticated native dispatch: PARTIAL

No dispatch implementation was completed in this run. No experimental image was
built and no 2-CPU guest was launched. Both operations, native rejection cases,
and resource retirement remain unproved. Production FastLane remains disabled.
This is not production readiness or routing security closure.

## Identity and audit

HEAD is `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`; the tree was extensively
dirty before this work. The source manifest in
`.fastlane-evidence/m61b-r1c/validation-20261103T233524.629538Z/source-before.json`
has SHA-256 `2376dcfd04b6de95a0de8a98cd0b57a344e185c0273486091b82507095c45071`.
It records tracked and nonignored untracked file contents excluding evidence,
not the build environment; it predates this report. The before and after
validation manifests match. The same directory contains the initial status,
tracked diff, command lines, timeouts and logs. Previous evidence was retained.

The smallest fixture hooks remain the authenticated `channel_fixture` supervisor
and Actor, its `pair` and `authorize` functions, and the M5.2 `shared_ring::Native`
backend. Kernel contract operation 31 resolves the paired peer and policy from
kernel state; operation 32 commits only after transport-specific readiness;
operation 35 checks the receiving identity and actual IPC reply target. These are
source observations, not receipts for this pilot. The trusted service operation
IDs are 1, 2 and 3, whereas the wire opcode is 0x10: a trusted, independently
checked binding remains necessary. The ring has a single payload-writing role;
a complete authenticated response path must also be designed and exercised.
The experimental features checked were `experimental-fastlane-authority` and
`fastlane_contract_test`. Existing R1B host and historical M5.2 results are
documented in `fastlane-m61b-r1b-operation-equivalence.md`; they do not prove this
fixture's native behavior.

## Validation and outstanding proof

Fresh host operation, IPC envelope, kernel policy/contract and parser tests
passed, as did the experimental IPC and kernel checks. Existing retirement
parser regressions and the ordinary release build passed. Exact commands and
results are in the validation summary. These checks were run without an image
or pilot; retirement parser checks are synthetic, not physical retirement.
The driver exits 1 intentionally for incomplete native acceptance. Reproduce
the checks from the repository root with
`python3 .fastlane-evidence/m61b-r1c/validate.py`.

Host tests exercise the deterministic reverse operation and distinct encodings;
they do not demonstrate application equivalence across live transports. No
native identity, policy, selection, operation, response, closure, or inventory
receipts exist. Incorrect incarnation, stale policy generation, unauthorized
operation, KernelOnly FastLane, Required downgrade, duplicate completion, and
peer closure during establishment are all NOT_RUN. No exactly-once physical
release or TLB acknowledgement is established; no remote target was observed
in this nonexistent pilot. No zero-inventory claim is made.

The default release ELF SHA-256 is
`b3aaa5d2bb13eccb6369ad296b2abf6d93e5886a7088db0f1665afdb99c84cfd`.
The targeted scan found none of `[FL-M61B-K]`, `/test/contract-audio`,
`policy_generation=`, `fastlane_operation`, or `[FL-NATIVE-RING]`. This is a
targeted exclusion check, not an exhaustive security audit.

The immediate missing executable proof is the authenticated Actor dispatch,
including an explicit operation-permission binding and complete response path,
followed by one source-bound two-transport 2-CPU pilot and its correlated
retirement controls. R2 should begin only after that proof and then expand the
adversarial and CPU matrix, including nonzero remote TLB acknowledgement.
