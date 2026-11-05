# M6.1B-R1B operation equivalence: PARTIAL

This continuation implements a bounded operation wire format, register-IPC
envelope and application outcome bookkeeping. It does **not** implement the
authenticated native fixture dispatch. The 2-CPU R1B pilot and all ten native
negative controls are **NOT_RUN**. No native operation equivalence, failed
establishment cleanup or physical retirement is claimed. Production FastLane
remains **DISABLED**. This is not production readiness evidence.

## Source identity and preservation

HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
Evidence root: `.fastlane-evidence/m61b-r1b/`.
`initial-status.txt`, `initial.patch` and `source-before.json` preserve the
starting dirty tree. The initial manifest hashes 11,021 tracked and nonignored
untracked files outside the evidence tree; its SHA-256 is
`d63b857c5a0b5e0a1a8a8bb1b687a3345d6a23a0ee8004c0f978df5731093a54`.
Canonical source identity is HEAD plus the path/content-hash manifest, not HEAD
alone. It does not attest the build environment or ignored files.

The final code validation is in
`validation-20261103T232813.680187Z/`. Its source manifest SHA-256 is
`b8418b25762b53a8e551e9b1a70e9f2222f707861204223eea661e4cdf983d8e`;
before/after validation identities match. This report was added afterward;
`source-final.json` records the delivered source including this report.
The earlier `validation-20261103T232652.787903Z/` pass is retained and covers the
protocol before the IpcMsg envelope adapter was added. Both validation drivers
exit 1 deliberately because native acceptance remains PARTIAL; neither contains
a failed native guest. No image was built for R1B, no QEMU guest was launched,
no full CPU matrix was started, and no commit or push occurred.

## Source and authority audit

The R1 report and experimental implementation were inspected alongside the
M5.2 shared-ring, M4 Doorbell and M2 restricted-mapping implementations. Key
source hashes are recorded in `audit-sources.json`.

The smallest existing pair to extend is the channel fixture's userspace
supervisor (original caller) and separately spawned `Actor` (service), both
using ordinary Account authorization. `channel_fixture::pair` and `authorize`
reuse the M1 pairing/receipt flow; `shared_ring::Native` supplies the M5.2
hostile-memory backend. The trusted `/test/contract-system`,
`/test/contract-audio`, `/test/contract-protected` and
`/test/contract-required` paths already embed the session-race binary in
experimental kernels. Syscall spawn assigns `contract_service` from the actual
embedded path. That binary currently has no R1B operation dispatch. No live
caller/service PID, incarnation or VM-generation witnesses exist for R1B.

`channel_contract_native.rs` derives `RecipientIdentity` from the current
process and the actual channel peer. It checks creator membership, live paired
state and exclusive VM membership before lazy issuance. The eight-slot catalog
belongs to `ChannelTable` under existing broker serialization. Resolution checks
policy ID/generation, complete peer identity and trusted role; exit/exec retire
records. Its checked generation allocator is separate from the channel allocator.
No catalog, authority, lifecycle hook, syscall or lock was changed here.

Transport policy does not itself authorize the wire operation. Existing
`Service::operation_authorized` permits role-specific operation IDs 1, 2 or 3;
the new wire opcode `0x10` is **not** a new kernel permission. No native caller
may treat codec success or `Operation::committed` as authorization. Integrating
the logical operation with a trusted fixture's existing operation permission
remains required; no generic minting API or permissive authorization was added.

The existing adapter's establishment commit is successful `Contract::finish`
in kernel operation 32, after transport-specific readiness checks. For Kernel,
these include the exact consumer receipt; for FastLane, both native roles and
all mappings. This is a source-level commit point, not an observed R1B receipt.
Its `event=terminal reason=success` means establishment success and must not be
counted as application completion. Existing contract state freezes transport
after commit. Recovery fallback after partial allocation is not implemented.

The existing M5.2 ring has one payload-writing role and a separate acknowledgement
page. Existing benchmark responses use ordinary IPC; they do not establish a
ring response implementation for this operation. The native pilot must specify
and authenticate its complete response path before claiming equivalence.

## Exact implemented operation sub-contract

`TRANSFORM_16`, opcode `0x0010`, version 1. Input is 0–16 application bytes;
output is those bytes reversed, with the same nonzero u64 request ID. Empty input
returns empty output. A request ID is scoped to one committed connection, which
accepts exactly one operation. This fixture has no durable/reconnect deduplication.

The actual IPC register transport has **four** u64 words, despite IpcMsg's
eight-word storage array (`kernel/src/ipc/message.rs`, `IPC_REG_WORDS` and
`validate_ipc_msg`). Two words remain for payload after framing, so 32 application
bytes would exceed this contract's conventional IPC capacity.

| Offset in 32-byte frame | Meaning |
|---|---|
| 0–1 | Little-endian operation ID `0x0010` |
| 2 | Version 1 |
| 3 | Kind: 0 request, 1 response |
| 4 | Application length, 0–16 |
| 5–7 | Reserved, must be zero |
| 8–15 | Little-endian nonzero request ID |
| 16–31 | Application bytes followed by mandatory zero padding |

IPC carries these bytes as exactly four little-endian words with zero transferred
capabilities. Request label is `0x6b010010`; response label is `0x6b020010`.
Label and kind must agree. The separate ring decoder accepts exactly a 32-byte
**private snapshot** from a validated hostile ring lease. Neither decoder
accepts PID, UID, service identity, policy identity or transport authority from
application bytes. A ring snapshot is not authenticated merely by decoding it.

Golden request: ID `0x0807060504030201`, payload `abc` gives words
`[0x0000000300010010, 0x0807060504030201, 0x636261, 0]`.
Expected response is the same ID and `cba`, with response kind/label.

`Frame::request` rejects zero IDs and oversized input. Both decoders reject
unknown opcode/version/kind, excessive length, nonzero reserved fields,
noncanonical padding, missing or extra frame bytes/words and zero IDs. The IPC
envelope additionally rejects wrong labels, counts and capability transfers.

Application results are local Rust results, not a newly invented wire-error
protocol: `Format`, `Duplicate` or `Terminal`. Malformed input is to close the
attempt with Failed through `Operation::fail`; no response or automatic retry is
defined for it. A response presented as a request also causes Failed. A valid
execution records Completed once. Repeating the recorded request ID returns
Duplicate; a different late ID returns Terminal. Rejection cannot overwrite the
first terminal outcome, and changing transport is not exposed by this object.
These are application bookkeeping properties; they do not serialize a kernel
publication race or authenticate who invokes the code.

Required native cancellation semantics remain: before establishment commit,
cancel the attempt once, publish no operation and fully retire partial resources;
after publication, reject cancellation/fallback and retain the original outcome.
The new application object starts after commit and implements **no** precommit
cancellation or establishment state. These native semantics are unimplemented.
FastLaneAllowed may choose IPC initially under the existing contract, but failed
FastLane setup must be physically retired before any fresh IPC attempt. No such
recovery is supplied by this protocol module.

## Focused results and limitations

`ipc/src/fastlane_operation.rs` supplies separate word and byte codecs and shared
deterministic operation logic; `fastlane_operation_ipc.rs` adapts actual IpcMsg
envelopes. They compile only in IPC tests or with the existing experimental
authority feature. No userspace service calls them yet.

| Stage, in requested order | Result |
|---|---|
| Host operation semantics, all lengths 0–16 | PASS: response oracle, ID/kind, duplicate/late rejection, terminal state, frozen transport, golden layout |
| Actual kernel policy/contract | PASS: 6 existing tests |
| Wire/envelope parser negatives | PASS: 2 tests; malformed lengths, caps, labels and headers |
| Experimental IPC and kernel type-check | PASS |
| One bounded 2-CPU R1B pilot | NOT_RUN; 0 native requests, 0 native responses |
| Focused retirement parser regressions | PASS: 21 existing tests; synthetic parser controls only |
| Ordinary release build / targeted marker exclusion | PASS |

There are six operation/envelope tests total. Comparison exercises distinct
framing implementations and the application outcome contract, with an independent
expected-response assertion. The transform function is shared. These host checks
are **not** execution through two actual transports, independent operation
implementations, or authenticated native request/response counts. No native R1B
receipt grammar/parser has been implemented; wire parser tests do not substitute
for it.

| Required native negative control | R1B result / narrower host evidence |
|---|---|
| Forged caller | NOT_RUN; existing catalog tests reject foreign service identity |
| Unauthorized operation on permitted transport | NOT_RUN; existing contract test rejects independently |
| Stale policy generation | NOT_RUN; actual catalog host rejection after reuse |
| Wrong channel generation | NOT_RUN |
| Duplicate operation publication | NOT_RUN; application bookkeeping rejects repeated request |
| FastLaneRequired downgrade | NOT_RUN; contract host rejection |
| KernelOnly FastLane | NOT_RUN; contract host rejection |
| Cancellation before commit | NOT_RUN |
| Peer exit during incomplete establishment | NOT_RUN |
| Fallback after publication | NOT_RUN; no transport setter in application object |

No native invalid attempt has an exactly-one-terminal-outcome witness. No native
publication, failure injection, late publication or cleanup race was exercised.

## Physical retirement and remote TLB evidence

There are no R1B native channel objects to correlate. Policy/channel binding,
grant creation/consumption, backing allocation lifetime, views, unmap initiation
and completion, target/ACK set, exactly-once physical free and final scoped
inventories are all **UNPROVED for R1B**. Absence of a pilot is not a zero-inventory
or frame-reclamation witness.

`historical-retirement-index.json` preserves hashes and paths for archived M5.2
2-CPU mapping/physical-object/ACK evidence. Its serial log contains 22 nonzero
remote-target requests and 22 ACK rows; the archived validation reports PASS.
These counts alone do not prove correlation/order for new code. Existing
retirement parser regressions separately reject missing/wrong ACK, early free,
wrong backing identity, duplicate physical free, changed VM/incarnation and
missing unmap. They distinguish zero-target unmaps from actual remote ACKs.
Neither the archive nor these synthetic tests closes R1B retirement proof.

## Production exclusion and reproduction

Default release ELF SHA-256 from final code validation:
`b3aaa5d2bb13eccb6369ad296b2abf6d93e5886a7088db0f1665afdb99c84cfd`.
The targeted scan found none of `[FL-M61B-K]`, `/test/contract-audio`,
`policy_generation=`, `fastlane_operation` or `[FL-NATIVE-RING]`. Default IPC
builds exclude the new modules; experimental kernel adapters retain their debug
feature gates. This targeted audit is not an exhaustive production security audit.

Reproduce from the repository root:

```sh
python3 tools/fastlane_m61b_r1b_validate.py
```

The driver records exact argv, timeouts, logs, exit codes, source hashes and
native NOT_RUN reasons in a fresh evidence directory. It runs the host operation
tests, kernel contract tests, parser tests, experimental checks, existing mapping
parser controls and ordinary release build in order. Exit 1 denotes incomplete
native acceptance even when every invoked check passes. It never builds or boots
an unrelated image to obtain a pilot receipt.

## Smallest remaining executable proof and R2

Add one opt-in dispatch to the existing supervisor/Actor pair, binding this
operation to the trusted service's independently checked existing operation
permission. Carry exact policy and channel references through authenticated
connection setup; implement and validate the complete request and response path
for IPC and shared ring. Add bounded kernel/service/caller receipts and a native
receipt parser before building one source-matched 2-CPU image. Then execute both
paths and all ten controls with attempt terminal receipts and object-by-object
physical retirement correlation. Native operation integration is the immediate
missing step; another host model does not close it.

Only after that pilot passes should R2 expand to the 2/4/12-CPU matrix. R2 also
retains any unproved nonzero remote-TLB ACK ordering, broader policy service
replacement/revocation behavior and production routing review. No production
routing, optimization, live migration or unrelated subsystem change is included.
