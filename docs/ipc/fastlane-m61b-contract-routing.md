# M6.1B contract routing: partial audit

Status: **PARTIAL**, not a qualified transport selector. No production readiness or performance claim is made. The working tree already contained the experimental implementation at the start of this audit. Its base commit is `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`; the tree is extensively modified, so the commit alone is not its source identity. The preserved initial snapshot and hashes are in `.fastlane-evidence/m61b/initial-source/` and `initial-source.json`; `audit-source-sha256.txt` identifies the files examined here. Evidence from earlier milestones cannot establish this changed source's native behavior.

## Authority and integration boundary

Endpoint creation returns an owner capability; nameserver registration is mediated in the kernel and checks endpoint ownership and registration authorization before deriving a send-only capability. Discovery alone is not a transport or operation grant. The service manager is itself a service and the nameserver registration path does not currently supply a trusted transport policy. A suitable experimental authority is therefore the kernel's existing trusted embedded-image launch path, with a policy catalog bound to the exact launched process identity. Production integration would require a separately reviewed trusted service-registration policy lifecycle, not a client-provided manifest or PID.

The existing authenticated channel is brokered by the trusted authentication/session roles. Channel records carry recipient identities including incarnation and address-space generation. The scheduler checks current identities and VM exclusivity; exec withdraws image roles and existing recipient-grant retirement and channel mapping retirement hooks handle lifecycle transitions. Those are the mechanisms to reuse, rather than minting a second kind of authority. Their interaction with this draft has not been proven by the M6.1B native tests.

## Existing draft and limitations

The draft in `channel_contract.rs` defines KernelOnly, FastLaneAllowed and FastLaneRequired, rejects unknown numeric values, defaults unspecified roles to KernelOnly, and separately checks fixture operation numbers. Kernel-assigned embedded paths identify system control, audio-like, protected and required-stream test roles; exec clears the role. The selection state records a single chosen transport and does not offer a replacement after it becomes ready. The native adapter attempts to reuse the authenticated channel and its bounded handle. A receipt's channel generation was previously labeled `pg`; it has been renamed `channel_generation` because it is not an independently established policy generation.

This is a draft, not a completed implementation. In particular:

* The named fixture paths all embed the existing session-race probe; no M6.1B service or workload implements the required connection protocol. Conventional operation equivalence and payload ordering are unverified.
* The static role catalog has no independently recorded policy identity/generation or policy update lifecycle. The current channel generation must not be interpreted as one.
* The adapter needs an end-to-end security review of operation authorization, publication boundaries, failed establishment cleanup, and exactly one terminal outcome per attempt, including errors before a contract enters the preparing state. Host state-machine tests do not exercise these paths.
* No native pilot, lifecycle/adversarial matrix, final scoped inventory, physical retirement or remote TLB acknowledgement proof was performed for M6.1B. No 2/4/12-CPU result can be claimed.
* The selection receipts are not yet a validated bounded receipt protocol for every outcome. Invalid accounting has not been integrated into selection, and must remain independent of operational transport selection.

The intended model is `Idle -> Preparing -> Ready -> Closed`, or `Preparing -> Failed`; any fallback must be resolved before channel setup and publication, and a ready connection must never change transport. The current model alone does not prove those properties across the native mechanisms.

## Checks and production boundary

The focused host contract model has four passing tests for catalog parsing, operation checks, the selection matrix and frozen state. The experimental kernel `cargo check` passed. `tools/test_fastlane_m61a.py` passed its three standalone tests and skipped 30 native-data-dependent tests; this is not an M6.1B parser validation. See `.fastlane-evidence/m61b/` for logs. `git diff --check` passed at audit time.

Experimental routing code is gated by `debug_assertions` and `fastlane_contract_test`; the latter depends on `fastlane_channel_test`, which has a non-debug compile error. The ordinary default-feature release kernel built successfully; the recorded string scan found no M6.1B receipt marker or fixture paths. The symbol scan is inconclusive because this release ELF contains no symbol table. These targeted checks do not prove every production behavior or constitute a boot test. Production FastLane remains disabled by the existing configuration and must remain so.

## Next executable milestone

Implement one bounded, authenticated 2-CPU fixture end to end, first defining a real immutable catalog identity/generation and an explicit operation-equivalence contract. Exercise successful KernelOnly and FastLaneAllowed selection, failed and cancelled setup, caller/service identity changes and post-publication downgrade rejection while independently checking all resource retirement and remote acknowledgements. Only after that pilot passes should the required adversarial suite and 2/4/12-CPU matrix run, followed by source-matched production exclusion. Preserve all failed attempts.
