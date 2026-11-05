# M6.1B-R1E — PARTIAL: reverse-channel establishment rejected

No native TRANSFORM_16 ring request or response was transferred. The bounded
2-CPU pilot reached authenticated request-channel attachment but failed before
mapping, ring activation, contract commit or publication. Native negative
controls were NOT_RUN because the positive pilot failed. Production FastLane
remains disabled. No commit or push was made.

## Implementation and exact blocker

An isolated `FL-M61E` fixture now activates `/test/contract-required` through the
existing Supervisor/Actor path. It attempts to use the existing M5.2 hostile
ring adapter for both 32-byte frames. Ordinary IPC coordinates setup/execution;
operation 35 retains its live kernel-resolved IPC caller authorization. No
kernel authority, mapping, Doorbell, or conventional operation code was changed.
The conventional `FL-M61B` dispatcher and operation remain intact.

The attempted return design uses a second M1 channel with roles reversed, with
its own one-use consent, attachment, three backing objects, six restricted
views, M4 notification and explicit retirement. **This design is not executable
under the current M1 invariant.** `ChannelTable::occupied` accepts any live
membership; `channel_propose` rejects an already participating producer with
`Error::Duplicate`. `channel_consent` independently imposes the same restriction.
The service is already the request consumer when it proposes the response ring.
No check was relaxed to bypass this rejection. The unexecuted setup and payload
code is implementation work in progress, not an accepted authenticated protocol.

## Native attempts and identities

Evidence root: `.fastlane-evidence/m61b-r1e/`.
HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
`baseline.json` records all dirty source hashes, initial status, and preserved
R1D receipt hashes; `baseline.patch` preserves the initial tracked diff.
Each build/native directory includes exact source copies, source hashes,
ELF/ISO and embedded service hashes, commands and artifact indexes.

| Attempt | Result |
|---|---|
| `build-20261104T084447.208107Z` | Build PASS |
| `native-20261104T084600.977399Z` | QMP socket sandbox denial; no boot |
| `native-20261104T084711.702964Z` | Actor CPU placement assertion; no channel |
| `build-20261104T084827.178280Z` | Rebuilt without optional Actor CPU pin; PASS |
| `native-20261104T084855.129758Z` | M1 rejects response-channel proposal; FAIL |

The CPU placement selector accepts only `session-racer`/`session-race-probe`
names; the trusted Actor is named `contract-required`. Removing its optional
pin argument fixed this fixture error without changing kernel placement checks.
The subsequent receipt observes consumer attachment on CPU 1, producer on CPU 0.

The final pilot's canonical source-map SHA-256 (sorted compact JSON) is
`3f78877fa23c367b4fe536f9dfe6fa18df4923c005e721e89944cb506d351b57`.
The first build's is
`c8129ac14b1d663cb0f3f340dce8c4b7173dfc24197869f11db9f0238b3b93c6`.
Archived manifests inherit the old driver's `milestone=M6.1A` label; directory,
commands and archived sources identify these as R1E attempts. The current driver
corrects that label; it has not been used to replace any archived receipt.

Final native receipts include:

```text
[FL-M61B-K] event=begin g=1 service=40 si=52 sv=89 caller=39 ci=51 cv=87 policy=2 policy_id=1 policy_generation=1 channel_generation=1 role=4 op=2 transport=2 fallback=0 phase=1 result=0 reason=contract
[FL-AUTH] op=3 pid=39 inc=51 vm=87 generation=1 result=0 cpu=0
[FL-AUTH] op=3 pid=40 inc=52 vm=89 generation=1 result=0 cpu=1
[FL-AUTH] op=0 pid=19 inc=31 vm=49 generation=0 result=3 cpu=0
[SESSION-E3-CLIENT] checkpoint=FAIL line=295
[FL-M61B-K] event=terminal g=1 service=40 si=52 sv=89 caller=39 ci=51 cv=87 policy=2 policy_id=1 policy_generation=1 channel_generation=1 role=4 op=2 transport=2 fallback=0 phase=3 result=5 reason=failure
```

The Account issuer is PID 19/incarnation 31/VM 49, session 1/epoch 1.
The caller is PID 39/incarnation 51/VM 87; the service is PID 40/incarnation
52/VM 89. Policy 1/generation 1 resolves from the trusted RequiredStream role,
independently of request channel generation 1. Their equal numeric generations
in this run do not imply a shared allocator. Policy is FastLaneRequired,
operation permission 2, transport FastLane, fallback false. The connection never
reached Ready. The sole kernel terminal is an establishment failure on caller
exit; there is no terminal application operation outcome.

## Validation, equivalence and retirement limits

Six operation/IPC codec host tests and seven contract/policy tests passed.
The M5.2 parser suite passed two active tests and skipped ten. Experimental
kernel type-check passed. These results do not establish native execution.
The production image audit passed in `production-20261104T085002.175827Z`;
ordinary release build and marker audit are recorded separately in
`release-build.log` and `release-exclusion.json`.

R1D's accepted conventional request `0123456789abcdef` and response
`fedcba9876543210` are preserved, with their original receipt hashes verified.
The attempted ring fixture uses the same Frame codecs and deterministic
transform, but no accepted ring receipt exists for native equivalence. The
trusted service-specific permission is 2 for RequiredStream versus 1 for
SystemControl; both resolve the same version/opcode independently of transport.
No claim is made that the transports operated within one connection.

All seven requested native negative controls are NOT_RUN: unauthorized
operation, stale policy generation, foreign/stale attachment, duplicate
completion, downgrade, peer closure before response and incomplete-establishment
cancellation. The observed reverse-offer rejection is a diagnosed setup blocker,
not a substitute for those controls.

No M2 backing was allocated in the final pilot. It supplies no grant/backing/
physical-frame lifetime join, no mapping/unmap receipts, no zero-target unmaps,
no remote TLB ACKs, and no exactly-once physical release witness. Caller exit
and a kernel contract terminal were observed; final scoped channel, waiter,
grant, receipt and mapping closure was not asserted. Session-subset cleanup
logs are not a substitute for those inventories.

Existing remote-ACK evidence remains separate and unmodified under
`.fastlane-evidence/m52/m2-remote-replay/native-20261103T075022.238344Z/`, including
its nested mapping run and `.fastlane-evidence/m52/audit/m2-remote-coverage.json`.
It is not promoted into an R1E physical-retirement result.

## Reproduction and next step

```sh
python3 tools/fastlane_m61b_r1e_validate.py --section build
python3 tools/fastlane_m61b_r1e_validate.py --section native --cpus 2 --timeout 180 --image-manifest <new-build>/validation-summary.json
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-ipc --lib fastlane_operation
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-kernel --lib capability::channel_contract::tests
python3 -m unittest discover -s tools -p 'test_fastlane_m52.py'
cargo check --locked --offline -p sunlight-kernel --features fastlane_contract_test
python3 tools/fastlane_m61b_r1e_validate.py --section production
cargo build --locked --offline --release -p sunlight-kernel
```

Use the archived driver/source snapshot to reproduce the original image identity;
new builds capture the current driver's corrected metadata. Native QMP requires
local Unix socket permission. Failed attempts have not been deleted.

Smallest executable next step: retain one authenticated request ring and use the
existing authenticated IPC reply target for the response, documenting this as a
mixed return design and PARTIAL under the both-directions ring requirement.
For full bidirectional ring acceptance, first design bounded duplex ownership
within M1 (including atomic pairing, role binding and joint retirement), then
run its affected native safety gates. Do not simply remove `occupied` checks.
M6.1B-R2 still requires accepted operation execution, all seven negative controls,
physical retirement/TLB evidence and scoped inventories before routing lifecycle
acceptance. This pilot establishes neither complete routing security nor
production readiness.
