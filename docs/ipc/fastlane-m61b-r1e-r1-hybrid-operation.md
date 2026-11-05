# M6.1B-R1E-R1 — PARTIAL; native hybrid operation PASSED

A real authenticated two-CPU guest executed one TRANSFORM_16 operation with a
native shared-ring request and an authenticated deferred conventional IPC reply.
The caller accepted `fedcba9876543210` for `0123456789abcdef`. Three physical
backing frames were released after six completed restricted-view unmaps and TLB
completion witnesses. All six remote target masks were zero. Production
FastLane remains DISABLED. This is neither duplex FastLane nor production
readiness. No commit, push, reset, clean, or evidence deletion was performed.

The milestone remains **PARTIAL**: hybrid-native stale-policy, invalid-direction,
caller-exit-before-reply and peer-closure-during-transfer controls remain unrun.
Host checks and independent IPC lifecycle regression results do not discharge
those controls. Complete final grant/receipt/waiter inventories are also absent.

## Source identity and preservation

Evidence root: `.fastlane-evidence/m61b-r1e-r1/`.
HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
`baseline.json` records the initial dirty tree and per-file source hashes;
`baseline.patch` retains its tracked binary diff. Its canonical source hash is
`5129c6fa27f0f52c08e30b7f19bbd9f6e0acdd4b6e27cb3740ff78f6da2e4ec5`.
Canonical hashes use SHA-256 of sorted compact JSON mapping source paths to
SHA-256 values, using the existing driver's source selection. This is a code
source identity, not a hash of all documentation or ignored evidence.

Initial positive image: `build-20261104T091419.927919Z/validation-summary.json`.
Initial positive guest: `native-20261104T091846.179440Z/`.
Canonical source hash:
`498976ea1261bf2c8796eda95b7b7996628165661ee09f76274dbef4fa95d595`.

Targeted-control image: `build-20261104T092119.627723Z/validation-summary.json`.
Targeted-control guest: `native-20261104T092147.417931Z/`.
Canonical source hash:
`bfa1079205d8f7c1a886674020512f373e5a90b89e17409846412772e242f8a7`.
Both guests verified source/ELF/ISO/embedded-service identity before launch.
Their directories retain source copies, commands, hashes, serial output, QEMU
output, validation manifests and artifact indexes. The second guest repeats
one positive operation and then runs the bounded controls; no contention matrix
was run.

`final-source.json` records final dirty identity. Final canonical source hash:
`994de8ad506263474ef1d4edb241a1b8069ee4b10c6c0d154a7af9ce1862e04d`.
Only the new receipt auditor and its integration into the new driver differ
from the targeted guest snapshot. Kernel, IPC, service and ring sources are
unchanged. `targeted-audit-final.json` applies that stricter auditor to the
original, unmodified serial evidence. The archived native driver's original
PASS used a weaker positive-marker check and is not alone the acceptance proof.

R1D's accepted conventional operation, R1E's failed reverse-ring attempt and
all prior evidence remain in place. `preserved-prior-evidence.json` records
hashes of their validation summaries. Existing unrelated dirty changes remain.

## Explicit directional contract and independent authorization

The trusted RequiredStream fixture accepts the exact new establishment encoding
`0x10202`: explicit hybrid bit 16, operation permission 2, required native offer
2. `Contract::hybrid_transform` records this experimental directional contract:

```text
Operation: TRANSFORM_16 (version 1, opcode 0x10)
RequestTransport: FastLaneRequired
ReplyTransport: KernelIPC
```

Only the exact encoding and trusted `/test/contract-required` service role can
select it. Other services or unknown directional encodings fail closed. Old
`0x202` and all old connection-wide policy selection semantics remain intact;
they do not acquire the hybrid flag. This is a fixture-specific authorized
contract, not a new general routing policy or an interpretation of
connection-wide FastLaneRequired as silently permitting IPC fallback.

The contract lives in the existing exact Channel row. The row binds the
Account-authenticated creator and consumer PID/incarnation/VM, opaque handle,
channel generation and establishment lifetime. The trusted service's catalog
reference independently binds policy ID/generation to the consumer identity
and embedded-image service role. Each operation resolves that reference again;
service exit or successful exec revokes it. Closing the channel closes its
contract. Neither a reference nor transport permission authorizes an operation.

`Service::wire_operation(1, 0x10)` independently maps to permission 2 for this
service. Establishment checks permission separately from transport selection;
operation 35 checks the exact wire identity, committed permission, actual
service identity, caller identity and service operation permission again.
Unknown versions/opcodes and reserved bits are denied. The control envelope
also binds handle, channel generation, wire operation and request ID 1. The
Actor requires the copied ring Frame to carry that same request ID.

## Reply authority and security/unsafe audit

Inspected mechanisms: `set_reply_target`, `defer_current_reply`,
`deferred_reply_endpoint`, `reply_target_is_live`, `complete_deferred_reply`,
`deliver_reply_to_target`, `finish_call`, `retire_exec_replies` and
`retire_exec_call` in `kernel/src/ipc/mod.rs`, plus process identities and
recipient-bound channel/role admission.

The caller makes a real synchronous, payload-free EXECUTE IPC call to the
Actor's endpoint after ring publication. That call creates an ordinary kernel
reply target through normal IPC delivery. No reply target is synthesized from
PID, UID, channel bytes or a guessed deferred token. The Actor's operation-35
check authenticates the actual kernel-resolved IPC requester against the exact
channel creator. This milestone adds an explicit `reply_target_is_live` check
there, before allowing operation execution. The kernel receipt joins caller
incarnation, endpoint and call generation to channel, policy and service
identity. Authorization and the subsequent deferral use existing scheduler
lifecycle serialization; no new lock or reply authority table was introduced.

After authorization, `ipc_defer_reply` transfers the real current target into
that service process's exclusive deferred queue. Its token is process-owned,
one-use and nonwrapping. The trusted Actor binds this token to its one operation
and request ID, copies the actual ring Frame privately, validates and executes
it, checks channel liveness, and publishes the Frame via
`ipc_complete_deferred_reply`. The kernel rechecks original caller incarnation,
endpoint, call generation, pending state and terminal state at publication.
A token owned by another process or an unallocated token cannot select a target.
The caller's successful synchronous return and exact decoded Frame comparison
are the acceptance witness. Actor publication logging alone is not acceptance.

Completion removes the deferred entry before delivery. A second completion is
an error; the live query becomes false. Cancellation/deadline/endpoint closure
withdraw pending calls and retained deferred targets through the existing
lifecycle machinery. Successful exec withdraws incoming and outgoing authority;
server exit cannot leave its token adoptable. These properties have independent
native kernel-method regression coverage in `ipc-regression/`.

Important limit: the generic deferred token is bound to IPC lifecycle, not to
an atomically checked ring-channel lifetime. Operation/channel binding beyond
that target is trusted Actor state and its prepublication channel check. That
check and completion are separate syscalls. This pilot does not prove that a
concurrent ring-only revocation between them revokes the deferred token. The
hybrid-native closure/exit controls and any required lifecycle bridge must be
completed before claiming the full Phase C closure property. Ordinary IPC reply
ownership/publication code was not weakened or bypassed.

No new unsafe Rust or raw-memory path was added. Requests use the existing
M5.2 `hostile::Producer/Consumer<Native>` backend: checked fixed geometry,
aligned raw loads/stores, x86 TSO/compiler barriers, no Rust reference to shared
bytes, and a bounded initialized private copy. VM withdrawal can terminally fault
an accessor, as documented by that backend. No kernel word-copy helper carries
the request. The EXECUTE IPC contains control identity only; response bytes use
the existing IPC Frame codec. M4 notification supplies no payload authority.
M1's `occupied` checks and one-live-channel-per-process invariant are unchanged.

## Actual native execution and outcomes

Both participants attached on different CPUs in the targeted guest. Caller:
PID 39/incarnation 51/VM 87. Service: PID 40/incarnation 52/VM 89. Account issuer:
PID 17/incarnation 29/VM 45. Channel generation 1; policy ID 1/generation 1.
Policy generation and channel generation use independent allocators.

Representative unmodified receipts:

```text
[FL-AUTH] op=3 pid=39 inc=51 vm=87 generation=1 result=0 cpu=0
[FL-AUTH] op=3 pid=40 inc=52 vm=89 generation=1 result=0 cpu=1
[FL-M61E] case=directional operation=65552 request=FastLaneRequired reply=KernelIPC fallback=0
[FL-M61E] case=request-published channel=1 caller=39 bytes=32
[FL-M61E] case=reply-created channel=1 token=9223372036854775809 request_id=1 operation=65552
[FL-M61E] case=private-copy channel=1 actor=40 authorized=1 bytes=32
[FL-M61E] case=actor-executed operation=65552 request=ring reply=ipc
[FL-M61E] case=duplicate-reply terminal=rejected count=1
[FL-M61E] case=response-published channel=1 transport=ipc request_id=1 bytes=32
[FL-M61E] case=acknowledged request=0123456789abcdef response=fedcba9876543210 terminal=success
```

The kernel authorization receipt additionally binds endpoint 36, call generation
6 and call incarnation 51 to the above identities. The complete serial record
contains all six restricted mapping receipts, native ring activation, contract
readiness, closure, physical retirement and reaping. Exactly one acknowledged
operation result is present. The existing kernel contract `event=terminal`
receipt describes establishment readiness, not a second operation result.
R1D's preserved conventional operation used the same wire codecs and transform.
There is no response ring, fallback, migration or duplex operation.

## Negative controls and validation scope

| Required control | Actual result |
|---|---|
| Forged/foreign caller | Native PASS: independent PID 41 forwards a real IPC call to the service; operation 35 denies it although the channel handle is known |
| Unauthorized operation | Native PASS: unsupported versioned opcode denied by operation 35; independent permission checks also pass host tests |
| Stale policy generation | HOST PASS in catalog revocation/reissue test; hybrid-native NOT_RUN |
| Foreign/stale channel generation | Native PASS for changed generation; foreign channel membership remains checked; exhaustive foreign-handle matrix NOT_RUN |
| Forged reply target | Native PASS: invented token denied without consuming the legitimate ordinary control target; original caller also cannot complete the service's token |
| Duplicate or late reply | Native PASS: completed token denied immediately and again during a later live control call; no second caller result |
| Caller exit before reply | Hybrid-native NOT_RUN; independent IPC lifecycle gate is separate evidence |
| Peer closure during transfer | Hybrid-native NOT_RUN; generic channel liveness check is not an atomic closure proof |
| Attempt to open second ring | Native PASS: both live participants' Account proposals return Duplicate, with no second backing allocation |
| Invalid directional contract | HOST PASS for unknown combinations, service mismatch and frozen lifetime; hybrid-native NOT_RUN |

Each native probe emits one rejection terminal receipt. The positive operation
remains the sole accepted operation result. These probes do not transfer more
payloads or leave a second operation/authority to adopt. The foreign Actor is
reaped, the original Actor is stopped/reaped, and the caller is reaped. No claim
is made about terminal/inventory behavior of the four unrun native cases.

Focused validation: eight contract host tests PASS; five reply-filter host tests
PASS (including two exact reply-identity tests); three deferred-token host tests
PASS; six codec tests PASS. The first `ipc::` host filter selected zero tests;
that retained log is not coverage. M5.2 parser suite: two active tests PASS,
ten skipped. Experimental kernel type-check PASS after the recorded formatting
error correction. The stricter hybrid auditor PASS and 19 mutated-transcript
parser controls reject missing native receipts, duplicate terminals, identity
mismatch, changed permissions/result, bad physical joins and unobserved remote
TLB claims. These mutations are parser checks, never native negative executions.

Affected native IPC regression: `ipc-regression/build-20261104T092440.954626Z/`
and `ipc-regression/native-20261104T092523.754385Z/` PASS on two CPUs. This covers
the existing nine deferred-reply cases plus four concurrent kernel-method reply
races and withdrawal methods. Its own parser explicitly keeps authenticated
Account peers, real exec and numeric PID reuse outside that gate's scope. It
does not replace the missing hybrid operation controls. Broad R2 testing was
not run.

## Physical retirement and TLB witnesses

The targeted guest maps three backing pages into both processes. Producer
rights are RW/RO/RW and consumer rights RO/RW/RO, all NX. The auditor joins each
map to the kernel-authenticated PID/incarnation/VM, channel generation, region
and physical frame. It requires this order for both views of each region:
backing allocation, mapping, unmap request, TLB request/completion, unmap
completion, then exactly one physical release.

| Region | Physical frame | Backing sequence | Release sequence |
|---|---:|---:|---:|
| Producer metadata | 532078592 | 1 | 18 |
| Consumer metadata | 532090880 | 3 | 27 |
| Payload | 532094976 | 5 | 36 |

Six unmaps completed. TLB request IDs 154–159 each have matching completion
receipts, **targets=0**, and no remote ACK. The physical witnesses are distinct
from the final `[FL-CENSUS] checkpoint=690 objects=0 views=0 frames=0` assertion.
Channel live count zero is separately asserted through operation 6. The caller
cleanup receipt reports owned endpoints 0, no pending call/reply target,
deferred targets 0 and pending commits 0. Completed token liveness is false.
This does not provide a complete joined final grant/receipt/waiter inventory.

Remote-ACK validation remains a separate proof. Prior M5.2 evidence at
`.fastlane-evidence/m52/m2-remote-replay/native-20261103T075022.238344Z/` remains
untouched and is not promoted into a hybrid-operation remote-ACK result.

## Failed/unbooted attempts and production exclusion

Retained attempts include `check.log` (CapabilityToken Display error),
`build-20261104T091405.165488Z/` (eight-word IPC storage compared with four-word
array), and `native-20261104T091447.949991Z/` (sandbox QMP socket denial, no boot).
The corrected check is `check-2.log`. `build-20261104T092053.573269Z/` succeeded
but was never booted: inspection corrected negative expected error codes before
building the targeted image. The IPC regression driver rejected an unsupported
`--timeout` argument before creating a guest; `ipc-regression/argument-failure.txt`
records it. No failed/unused evidence was replaced or deleted.

Production image audit PASS in `production-20261104T092635.076922Z/`.
Ordinary `cargo build --locked --offline --release -p sunlight-kernel` PASS in
`release-build.log`. `release-exclusion.json` hashes both ELF files and verifies
absence of hybrid, contract, native-ring, mapping, TLB and physical-frame markers.
The new contract/native adapter remain behind test/debug feature gates. There
is no production routing enablement. `git diff --check` retains one preexisting
unrelated Cargo.toml EOF blank-line finding, recorded in `diff-check.log`.

## Reproduction and smallest remaining proof

From repository root, with existing offline dependencies and Limine artifacts:

```sh
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-kernel --lib capability::channel_contract::tests
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-kernel --lib reply
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-kernel --lib deferred
cargo test --locked --offline --target x86_64-unknown-linux-gnu -p sunlight-ipc --lib fastlane_operation
python3 -m unittest discover -s tools -p test_fastlane_m52.py
cargo check --locked --offline -p sunlight-kernel --features fastlane_contract_test
python3 tools/fastlane_m61b_r1e_r1_validate.py --section build
python3 tools/fastlane_m61b_r1e_r1_validate.py --section native --cpus 2 --timeout 180 --image-manifest <new-build>/validation-summary.json
python3 tools/fastlane_m61b_r1e_r1_receipt.py .fastlane-evidence/m61b-r1e-r1/native-20261104T092147.417931Z/2cpu.serial.log
python3 tools/fastlane_native_boundary_validate.py --section build --reply-race --evidence-root .fastlane-evidence/m61b-r1e-r1/ipc-regression
python3 tools/fastlane_native_boundary_validate.py --section native --reply-race --cpus 2 --image-manifest <new-regression-build>/validation-summary.json --evidence-root .fastlane-evidence/m61b-r1e-r1/ipc-regression
python3 tools/fastlane_m61b_r1e_r1_validate.py --section production
cargo build --locked --offline --release -p sunlight-kernel
```

QMP requires permission to create a local Unix socket. Archived builds/guests
are tied to their original source snapshots; use new manifests for rebuilt
sources. Production generation intentionally replaces staged embedded service
artifacts, so rebuild an experimental image before a subsequent experimental run.

Smallest remaining executable proof: extend the existing bounded fixture with
a supervised, authorized caller that can exit while the service holds its real
deferred target, and a service/peer closure during ring acquisition. Require
exactly one terminal result and joined zero resource inventories in each case.
Exercise stale policy generation and invalid directional selection natively.
Resolve the ring-only revocation versus IPC completion serialization gap if the
contract requires token withdrawal on that event; do not merely trust a
precompletion userspace check as atomic authority. Retain remote TLB ACK
validation as separate remaining proof for this zero-target pilot.

R2 must still cover concurrent cancellation/closure/exec and publication,
incarnation and policy/channel-generation reuse, hostile Frame/control mismatch,
resource exhaustion and nonadoptability, complete scoped inventories, and remote
mapping retirement under contention. No performance, adaptive routing or
production security acceptance follows from the passing positive operation.
