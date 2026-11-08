# FastLane data plane audit — 2026-11-08

Scope: current working tree at HEAD `a8e2a3b18`. Read-only implementation audit;
no runtime changes. An existing modification to `rappid-rabbit/src/main.rs` was
left untouched. Archived guest results were not treated as fresh validation.

Verdict: PARTIAL. The experimental restricted channel has strong admission and
bounded backing, but the requested production, isolated, zero-copy transport
is not delivered by the current implementation.

## Findings

1. **High — legacy SHM has no session allocation budget.**
   `kernel/src/memory/shared.rs:49` rounds sizes with checked arithmetic and
   limits each object to 4 GiB, but does not charge a session or enforce an
   aggregate byte limit before allocating frames. `pmm.rs:186` accounts frame
   ownership/class and rejects exhaustion, rather than enforcing a session
   budget. Repeated allocations can consume available memory; large requests
   also allocate while the syscall holds scheduler, PMM and broker locks.
   Enforce cumulative session/process limits and reserve budget before backing
   allocation, rolling charges back on failure. The experimental channel itself
   is bounded to eight rows and three pages per row; this finding concerns the
   exposed legacy allocation path, not an unbounded channel geometry.

2. **High requirement gap — legacy mapping does not enforce two endpoints.**
   `shared.rs:177` accepts a live legacy token without checking receiver identity
   or membership. `capability/mod.rs:1208` resolves it as bearer authority.
   A third process holding that token can map the object. This is legacy API
   behavior, not a demonstrated bypass of restricted FastLane objects:
   restricted objects explicitly fail legacy resolution. Use the authenticated
   channel path for this isolation requirement; do not build it on `shm_alloc`
   plus token distribution alone.

3. **High delivery gap — native transport remains debug gated.**
   `capability/mod.rs:19` gates mapping on debug assertions and
   `fastlane_mapping_test`; native ring activation is likewise gated by
   `fastlane_shared_ring_test`. The native backend lives in
   `services/sunlight-uac/src/bin/channel_fixture/shared_ring.rs`, not a verified
   RapidRabbit/audio production connection. Production readiness cannot be
   inferred from the fixture. Existing M5.2 documentation also explicitly states
   production FastLane is disabled.

4. **Medium requirement gap — native shared-ring receive copies payload.**
   `fastlane-ring/src/hostile.rs` deliberately returns initialized private copies
   because a producer retains a writable payload alias. The native fixture uses
   this adapter. Cooperative zero-copy leases exist in `fastlane-ring/src/leases.rs`,
   but are explicitly unsuitable for hostile native mappings. Do not substitute
   those reference-based leases to claim zero-copy across untrusted processes.
   Decide whether bounded private copies satisfy the product contract or design
   kernel-enforced slot ownership before claiming strict zero-copy.

## Checklist assessment

| Requirement | Assessment |
| --- | --- |
| SHM API and flags | `SHM_PAGE` is the 4096-byte size constant, not a permission flag. `shm_create` accepts flags, but `sys_shm_alloc` does not consume them; `sys_shm_map` currently maps legacy objects RW. |
| Exactly two authorized VMs | Present in experimental channels: immutable PID/incarnation/VM roles, session authorization, one-use consent/attachment, duplicate map rejection, and VM exclusivity. Absent from legacy bearer mappings. |
| Least-privilege mappings | Experimental pages separate producer cursor/descriptors, consumer cursor, and payload. Only the owning role writes each cursor; payload is producer RW / consumer RO, with NX mappings. |
| Atomic head/tail and false sharing | Cooperative ring uses `AtomicU64`, not the requested `AtomicUsize`. On its enforced x86-64 target both are 64 bits; cursors occupy separate 4096-byte-aligned pages, exceeding 64-byte separation. Native adapter uses aligned raw x86-64 word MOV accesses. Treat the type difference as an ABI choice to document, not evidence of a race. |
| Acquire/Release, mutex-free payload path | Cooperative leases use Acquire peer-cursor loads and Release publication/reclamation. Native adapter uses compiler fences with x86 TSO. No kernel payload mutex is used; setup and control operations still take kernel locks. |
| Quota | Fixed experimental geometry bounds backing to 96 KiB globally for eight channels, excluding page tables/metadata. Legacy session byte quotas are missing. The draft `data_plane::Plan` budget is a planner, not kernel enforcement. |
| Access tokens | Experimental operations check token/generation plus exact role identity. Legacy SHM is bearer based. |
| Process resource tracking | Both paths record local views in `Process::mapped_shared`; legacy owners also record `owned_shared`. |
| Crash / SIGKILL reclamation | Structurally present: external termination marks cleanup pending; reaper revokes endpoint/channel authority, drains restricted mappings, and calls `cleanup_shared_pages`. Shared release preflights frames, unmaps with synchronous TLB invalidation, then releases references. Invariant failures intentionally panic with backing pinned. Fresh guest fault/kill and remote-TLB tests remain necessary for runtime sign-off. |

Key cleanup references: `kernel/src/sched/mod.rs:3258`, `:2956`;
`kernel/src/memory/shared.rs:241`, `:411`;
`kernel/src/capability/channel_mapping.rs:37`.

## Validation performed

- `cargo test -p sunlight-fastlane-ring --target x86_64-unknown-linux-gnu`:
  42 unit tests and five documentation tests passed. Includes cooperative
  concurrency/rollover, hostile mutation, cancellation and geometry controls.
- `python3 -m unittest discover -s tools -p test_fastlane_mapping.py`:
  21 tests passed. These are synthetic trace-assessor rejection controls, not
  evidence that guest mapping or reclamation actually occurred.
- No fresh QEMU boot, SIGKILL/crash matrix, OOM attack, or native mapping/TLB
  reclamation run was performed. Host tests do not establish those properties.

Priority: enforce legacy session quotas; keep endpoint isolation on restricted
channels; settle the copy/zero-copy contract; validate a production integration
with fresh unauthorized-map, quota, crash, SIGKILL and SMP reclamation evidence.
