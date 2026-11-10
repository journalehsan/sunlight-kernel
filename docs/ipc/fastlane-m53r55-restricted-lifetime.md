# M5.3R5.5 restricted backing lifetime contract

Production FastLane and restricted admission remain disabled. This records the
existing debug-gated restricted implementation; it does not enable a production
policy. Evidence: `.fastlane-evidence/m53r55-20261110T080804Z/`.

Authentication authority ends when the exact Account registry row is removed
under the scheduler/broker lifecycle serialization. Active, unexpired membership
is required for new authenticated launch, allocation, proposal and consent.
Suspension also fences new authority. Stored process membership is an identifier,
not a surviving authorization grant. Session IDs/epochs use checked monotonic
allocation within one kernel instance; registry removal does not recycle them.
Account authorization is implemented in `kernel/src/capability/sessions.rs`,
`sessions/launch.rs` and `services/sunlight-uac/src/bin/uac_service.rs`.
SessionD's `begin_stop` revokes Account authority before stopping the shell.

Restricted mapping policy is forced revocation. `ChannelTable::retire_session`
and participant retirement mark channels terminal. An existing PTE can persist
until the ordinary mapping sweep, and must pin its backing during that interval.
`Scheduler::fastlane_drain_mappings` in `capability/channel_mapping.rs` fences the
SHM object, then drains all participant views. Logical authority revocation is
therefore distinct from completion of hardware access withdrawal. New mapping
admission calls the sweep and exact identity/role/session checks. A retired row
cannot recycle while it still has backing. This policy also applies to expiry,
participant incarnation/VM change and explicit channel revoke.

Physical backing remains owned by its actual view references. The channel's
backing token is cleanup metadata, not an additional reference. `SharedLifetime`
in `memory/shared_lifetime.rs` counts published views; its `retire` only fences
admission. The restricted adapter actively withdraws those views. These two
levels are consistent: the generic lifetime object does not itself edit PTEs.

`memory/shared.rs::free_shared_page` preflights the real PTE/frame and VM-region
ledger while serialized, removes the leaf through `AddressSpace::unmap_page`,
waits for its synchronous TLB completion, removes mapping metadata, then releases
the view reference. The final zero-view release removes the broker entry and
moves its frames and original quota receipt into `ReclaimedShared`. This move
is the precise event authorizing PMM reclamation. A zero-view revoked object may
also transfer ownership directly. A mismatch fails stop, retaining uncertain
backing rather than proceeding with cleanup.

`memory/tlb.rs` serializes invalidations and uses a mutation epoch to synchronize
VM activation. It targets CPUs actually retaining the exact VM, waits for the
request's acknowledgments, and only then returns. Zero remote targets requires
no fabricated remote witness. An inactive VM reloads current page tables at
activation. `ReclaimedShared::reclaim` in `capability/mod.rs` frees the frames,
then drops the move-only receipt. `memory/shared_quota.rs` refunds the immutable
original process incarnation/session payer exactly once. Session removal does
not migrate a receipt or refund it. Charges exist through legitimate final free.

Process termination, deferred safe cleanup and full reaping are separate scheduler
boundaries (`sched/mod.rs`). A pending cleanup callback owns the process record;
it does not own an extra SHM reference after its mappings have been withdrawn.
Later cleanup traverses remaining mapped/owned records and cannot release a
mapping already removed by forced revocation. Holding an owner at IPC receive
therefore does not require retaining restricted backing after both PTEs and
references have retired. Retaining another reference solely for that barrier
would change policy without a real ownership obligation.

The overall policy is mixed by resource type: restricted channels forcibly drain;
legacy bearer SHM preserves surviving peer mappings. The R3 security contract
requires fencing authority *before retiring mappings* and preserving charges
*while a peer retains backing*. It does not mandate graceful restricted retention.
R4's retained-backing accounting rule is likewise conditional on actual retention.
R54's requirement that restricted backing survive until held owner exit was a
fixture expectation mismatch. Its observed early free is correct forced
revocation when the exact view/TLB/free/receipt chain is present, not evidence
of a use-after-free. The original 28-case peer-retention acceptance is preserved
and cannot be relabeled PASS merely by adopting this classification.

No same-frame physical reuse proof or global memory zero-drift claim is made.
Native qualification and any missing session-generation boundaries are reported
separately from this contract and from host tests.
