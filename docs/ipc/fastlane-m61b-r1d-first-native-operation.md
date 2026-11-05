# M6.1B-R1D — PARTIAL; conventional IPC NATIVE-PASSED

The next-step conventional pilot passed on a 2-CPU QEMU guest. Durable evidence:
`.fastlane-evidence/m61b-r1d/native-20261104T082624.957671Z/`.
The source-matched image is recorded in
`build-20261104T081833.580185Z/validation-summary.json` under that evidence root.

Supervisor activation is `FL-M61B`, guarded by `fastlane-contract-test`.
The Supervisor spawns the trusted `/test/contract-system` embedded Actor,
checks its kernel IPC badge, obtains Account consent, begins and commits a
KernelOnly contract, and dispatches a Frame over conventional IPC. The Actor
checks operation 35 against the live IPC reply target before executing.
The caller checks the exact decoded response against the expected transform.
Serial receipts show actor execution and request `0123456789abcdef`, response
`fedcba9876543210`, followed by successful fixture exit and process reaping.
Channel revoke, Actor exit and zero live channel rows are asserted. This does
not prove FastLane physical backing retirement or remote TLB acknowledgement.
The kernel `terminal=success` receipt describes contract readiness, not operation
completion; the separate Actor and caller assertions establish execution.

Failed attempts were retained: two feature-table build errors, a sandbox QMP
socket denial, and a native assertion at the incorrect opcode-2 resource-create
call. The fix removes that call: consent already establishes the paired state
required for conventional begin/commit. No authentication check was weakened.

Reproduce:

```sh
python3 tools/fastlane_m61b_r1d_validate.py --section build
python3 tools/fastlane_m61b_r1d_validate.py --section native --cpus 2 --timeout 180 --image-manifest .fastlane-evidence/m61b-r1d/build-20261104T081833.580185Z/validation-summary.json
```

The validator PASS is scoped to conventional IPC only, not R1D acceptance.
Shared-ring operation, native negative controls, duplicate completion controls,
and physical retirement witnesses remain NOT_RUN. Production routing remains
disabled. No commit or push was made. The previous report below records the
prior attempt; its NOT_RUN statements are superseded for conventional execution.

---

# M6.1B-R1D — PARTIAL

No native guest was launched. There are no native execution, response,
retirement, inventory, or TLB receipts from this attempt. The requested milestone
is incomplete. No external authorization or toolchain blocker was established;
the remaining blocker is unimplemented dispatch integration.

Implemented: a trusted version-1 opcode-0x10 binding to existing service
permissions in `Service::wire_operation`. Unspecified services, unknown versions
and unknown opcodes are rejected. Native contract operation 35 now accepts the
exact packed version/opcode (0x10010), rejects high reserved bits, and checks
that the bound permission matches the committed contract. It independently
rechecks service operation permission using the kernel-resolved IPC requester.
Existing consumer identity, caller identity and catalog generation checks remain.
This changes the experimental operation-35 argument contract from a permission
number to a versioned wire identity; no Actor caller has been integrated yet.

HOST-TESTED: kernel contract tests, including version/opcode rejection and
protected-operation permission independence. Experimental kernel check and
ordinary release build passed. Targeted release marker exclusion results and
ELF identity are in `.fastlane-evidence/m61b-r1d/release-exclusion.json`.
Production routing was not enabled.

NOT_RUN: operation/parser suites, conventional native dispatch, shared-ring
request/response, 2-CPU pilot, native negative controls and retirement checks.
No physical reclamation or zero-inventory claim is made. Neither zero-target
retirement nor remote TLB acknowledgement was observed.

The inspected execution boundary is `channel_fixture::actor`, whose loop only
routes existing fixture commands and has no Frame dispatcher. `spawn` uses
`/bin/session-racer`, whereas trusted service identities are assigned to the
`/test/contract-*` embedded paths. Supervisor activation and a service-path spawn
must be connected to contract begin/commit and operation-35 authentication.
The M5.2 `shared_ring::Native` backend implements role-restricted Memory access;
a reverse response channel is not implemented here. Shared-ring expansion must
wait for a passing conventional native operation.

Smallest remaining executable step: wire a trusted service Actor into the
Supervisor, pair through Account authorization, commit a conventional contract,
validate 0x10010 through operation 35, execute Frame::transform, and verify the
reply and terminal outcome in a bounded 2-CPU guest. This work stopped before
that implementation; host checks do not substitute for it.

HEAD: eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6. Existing dirty files were retained.
Evidence includes initial status, binary tracked diff and before/after source
hash manifests. No commit or push was performed. No native failed/aborted
attempts exist because none was started.

Reproduction commands (repository root):

```sh
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-kernel --lib capability::channel_contract::tests
cargo check --locked --offline -p sunlight-kernel --features fastlane_contract_test
cargo build --locked --offline --release -p sunlight-kernel
```
