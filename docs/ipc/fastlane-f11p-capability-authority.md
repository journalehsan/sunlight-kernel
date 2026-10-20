# FastLane F1.1P: capability issuance and authenticated authority

Baseline: `4c70511d0`, 2026-10-20. This phase repairs ordinary bearer issuance;
it does not implement restricted SHM grants. The [F1.1 audit](fastlane-f11-grants.md)
records the before-state, including the reproducible XOR prediction attack.
Implementation and validation results will be recorded below after execution.

## Issuance inventory and design

The ordinary public ABI remains one u64: two class bits and 62 payload bits. Class tags
are routing information, not entropy or authorization. Before this repair,
ordinary issuance had no justified unpredictable-bit strength after observation
of related handles. Fresh conditioned payloads provide at most 62-bit bearer
guess resistance, not 128/256-bit handle security. Kernel ChaCha20 uses a 256-bit
key; truncating its output does not give a 256-bit token.

| Path | Before generation / uniqueness / failure | Issuer, holder and use checks | Publication, exposure and revocation |
| --- | --- | --- | --- |
| `CapabilityBroker::create_endpoint` | Counter XOR boot seed; infallible pushes; checked ID increment panics | Kernel/current process creates owner SEND_RECV bearer; receive/destroy require actual owner | Returns token in register; endpoint destroy/exit removes endpoint's tokens |
| `mint` | Same generator, Option only rejects wrong class/dead endpoint | Trusted kernel caller selects IPC rights; token possession checks SEND | Bearer may be copied via IPC; individual revoke removes one token |
| `derive` | Same generator; subset checked, exact-rights token reused | Existing source rights bound endpoint and attenuate new rights; no recipient binding | Not a revocation tree; destroy endpoint withdraws all derived tokens |
| `sys_shm_alloc` / `alloc_shared_region` / `mint_shared_region` | Same generator after backing/PTE allocation, infallible broker push | Kernel current process owns record by PID; any live bearer maps whole backing RW/NX | Complete owner view before publication; final verified view releases backing; retirement only fences new maps |
| `mint_shared_page` | Same mechanism, infallible single-frame vector | Trusted kernel/test helper | Must preserve caller ownership of frame on issuance failure |
| `grant_vfs_capability` / `sys_grant_capability` | Same generator; 64-record cap, infallible push | Kernel authenticates the full-width broker caller PID (no u32 truncation); prefix/flags checked on use; bearer holder not separately bound | VFS and SHM share tag but separate tables; do not log full token |
| `grant_elevated_vfs` | Multiple mints, no rollback if later prefix fails | Trusted kernel policy, base exec prefixes | Must atomically reserve or roll back all prefix records |
| `mint_account_grant` | Fresh entropy, checks collision only in account table, 64 limit, infallible push | Account GrantBinding plus expiry/single-use consumption | IPC tag overlaps other grant tables; consume removes even mismatched grant |
| `mint_auth_session_grant` | Fresh entropy, no collision check, 64 limit, infallible push | Trusted authenticated path, owner PID, uid/gid, expiry, consume once | Owner binding is PID, not full future memory-grant identity; exit now removes owned grants |
| Wise Owl delegation/session authority proofs (`delegated_caller`, syscalls 142–145) | Already two fresh u64 words; previously no duplicate-pair check and infallible Vec insertion | Kernel captures live caller/mediator VM generations, uid, session and purpose; trusted validator consumes once | Separate 32+32 pair tables; now reject zero/duplicate pairs across tables and reserve fallibly; bounded one-attempt issuance fails closed |
| Native file descriptors (`process/fd_table.rs`, `sys_open`/`sys_close`) | Predictable per-task table index, not a bearer secret; 256 slots | Authenticated current/shared process FD table plus descriptor rights; kernel VFS handle remains behind that table | Slot reuse is intentional after consuming close; backend open is closed if FD publication fails; no crypto-token ABI change |
| Legacy VFS daemon OPEN/READ/CLOSE (`services/vfs_server`) | Raw u32 backend identifiers, not broker CapabilityToken | Existing compatibility service does not bind its open metadata to recipient incarnation; writes mostly reject unknown actor, with legacy localtime exception | Audited only. Kernel issuance repair does not secure this separate object-ID protocol or promise its generation nonreuse; do not use it to establish FastLane memory authority |
| `check`, `token_owner`, `resolve_shared_region`, `resolve_vfs_capability` | Equality/table lookup; type gates IPC/VFS, SHM separate table | Bearer lookup is not proof of intended recipient; receive/management retain owner checks | Invalid/type-confused handles must never create records |
| IPC `enqueue_call`, nameserver mediation | Copies token words, kernel overwrites sender badge with caller PID | Private SEND relies on secret possession; receive ownership and sender identity independent | Nameserver public SEND intentionally shared; public lookup is not a private authority boundary |
| `revoke`, `destroy_endpoint`, `revoke_endpoints_owned_by`, reaper | Removes IPC records; SHM peers retain valid views after owner loss | Scheduler authenticates cleanup owner | Old IPC handles fail lookup; SHM retirement preserves backing until verified release. Owner-bound auth/account grants are removed; transferable VFS bearer lifetime is preserved (64-record bound) |

Normal logs must not contain complete tokens. Scheduler and PTY watchdog diagnostics now report capability/message presence instead of dumping bearer words or IPC payload capability arrays. The spawn `caps` log is a service permission bitmask, not a capability token. Register returns and explicitly
delegated IPC payloads intentionally expose bearer values to their recipients.
The public spawn selector and nameserver bootstrap selector are protocol
selectors, not secret bearer authority; their independent policy checks remain.

## Entropy readiness

`entropy::init` runs after heap/PMM, before capability self-tests, Init, VFS,
SunlightD or SHM publication. It first tries host-backed virtio-rng, then RDSEED,
then RDRAND; the CPU/hypervisor entropy source is trusted. TSC/timing is excluded.
The existing stream has 40 seed bytes, ChaCha20 with a 256-bit key, 64-bit block
counter and 64-bit nonce. Source qualification is a trust policy, not a measured
entropy estimate. `rand_service` is not used to bootstrap the kernel.

The repair must refuse minting if the conditioner is unavailable or exhausted;
no counter/zero fallback. Early boot must stop before publishing user authority
if entropy is unavailable. The conditioner now has a known-answer vector and counter exhaustion check. Full output is preflighted before mutation; exhaustion marks readiness false and all subsequent requests fail closed. Reinitializing an already ready stream is rejected by returning the existing source. Successful virtio seed collection resets the device, clears the seed buffer and returns DMA frames only after reset acknowledgement; timeout paths conservatively retain potentially DMA-visible backing. Queue size is checked against its fixed allocation and partial allocation failure releases its unsubmitted frames.
The ChaCha block function is specified in
[RFC 8439 §2.3](https://www.rfc-editor.org/rfc/rfc8439.html#section-2.3);
this kernel uses the original 64-bit counter layout rather than the IETF
96-bit nonce stream layout. A matching block-state known-answer vector can
still validate the round function. Seed unpredictability is essential; see
[RFC 4086](https://www.rfc-editor.org/rfc/rfc4086.html).

## Fallible publication contract

Under CAP_BROKER, first check bounded active-record capacity and reserve the
destination Vec(s), then draw at most 16 random candidates. Reject reserved
values/zero payload and equality with **any active token in any broker table**,
including account/session grants and retired SHM. Never overwrite authority.
Return explicit entropy, collision, capacity or endpoint-ID exhaustion errors.
The existing broker is the registry; do not add an unbounded token-history set.

Endpoint publication commits endpoint and owner capability together after both
reservations and candidate selection; failure consumes neither endpoint ID nor
record. Derivation preserves rights checks and exact-rights reuse. VFS batch
issuance rolls back earlier records on failure. SHM prepares a token and broker
slot before allocating frames; failed VM/backing setup consumes entropy but
publishes no authority. Commit consumes the private preparation only after all
owner PTEs exist, with an exclusive borrowed preparation guard that keeps the broker (and its caller's lock guard) borrowed until commit. Rust therefore prevents intervening issuance or dropping that lock while a preparation is live. Dropping the preparation on rollback publishes no record.
Single-frame helper failure leaves the physical frame with its caller.

Lock order remains SCHEDULER → PMM → CAP_BROKER → entropy conditioner on SHM
paths; endpoint creation must authenticate the current process under SCHEDULER
through publication. Entropy initialization acquires no broker lock, and
runtime entropy generation acquires no scheduler/PMM lock. No readiness waits
or unbounded random retries occur under the broker.

Active equality exclusion is deterministic; broker diagnostics count prepared random candidates (including candidates discarded by SHM rollback), collisions, entropy failures and exhausted retries; they do not mislabel every prepared candidate as a published grant.

Active equality exclusion is deterministic; resistance to guessing/reissuing a
previously revoked bearer is probabilistic within the 62-bit namespace. No
unbounded history is retained. A later memory grant must additionally bind
object/grant generations and recipient incarnation; random possession is not
a substitute. With n live same-tag tokens, q independent guesses have at most
approximately q*n/2^62 success probability. Unfiltered random collisions over k
draws have birthday probability approximately k*(k-1)/(2*2^62), but active
collisions are rejected before publication. Bounded retry exhaustion fails
closed even for a stuck or malicious source; these tests do not prove source
quality statistically.

## Authority beyond token secrecy

F1.1P preserves bearer semantics and current owner/badge checks. Future restricted
create/issue/map/delegate/revoke/close operations must bind exact nonzero live
kernel process incarnation, exact address-space root/generation (including exec),
original issuer/lifecycle authority, recipient identity, rights/delegable rights,
object and grant generations, and revoker authority. PID/name/header cannot
provide those credentials.

Native threads and supported Linux clone share PTEs and VM identity. Granting
one task a mapping grants memory access to every task sharing that VM, including
future borrowers. Future restricted grants must use VM-wide authority or
explicitly exclude borrowing before and after grant creation. This phase changes
no threading behavior and exposes no restricted mapping API.

## Results and handoff

Pending implementation and fresh evidence under `target/fastlane-f11p-evidence/`.
F1.1 restricted rights, legacy provenance fencing, active PTE withdrawal,
generation-bound grants and native RO/post-revoke faults remain outstanding.
F1.2 ring/Doorbell remains deferred.

## Validation scope and residual limits

The security fixture uses separate ring-3 owner, explicitly authorized reader,
and attacker address spaces. Only the owner and reader receive private handles;
attackers obtain their own endpoint/SHM handles through actual syscalls, then
try the historical 62-mask prediction against each authority class. Authorized
legacy SHM read/write and IPC request/reply (including the kernel sender badge)
are checked independently. Each stress iteration creates an endpoint, derives
SEND, rejects RECEIVE/destruction with that SEND handle, destroys via owner
authority and rejects stale SEND. Two workers per configured CPU use existing
affinity masks. Test entropy overrides are instance-local and compiled only in
host tests or `cap_issuance_test`; the global broker never receives them.

Host tests compile the actual broker and actual conditioner. Hardware-dependent
entropy input and display-authority callbacks are replaced only in that host
harness. A deterministic host seed verifies behavior, not entropy quality or
statistical security. Native boots use actual qualified kernel entropy, and a
separate no-device/no-RDSEED/no-RDRAND boot verifies fail-closed bootstrap.

Native samples report 128 local-broker create/mint timings in guest TSC units
(p50/p95/p99/max). Create includes existing UART logging. They exclude syscall
entry and broker-lock acquisition. TCG timing, concurrent host workload and
small sample count prevent a production latency or contention claim. No
before/after boot speed, steady-state IPC gain, concurrent issuance throughput,
failed-cleanup latency distribution or FastLane throughput is claimed. Native
SMP operation counts are correctness stress measurements.

Known limits: active-token equality is guaranteed within the production broker;
revoked-token recurrence remains a bounded-width probabilistic risk. Expired
session grants occupy their bounded slots until consumption/owner exit; account
grants prune expiration on mint. Transferable VFS capabilities retain their
existing lifetime across issuer exit and have a 64-record bound. The legacy
VFS daemon's raw object-ID protocol is audited only, not converted into secret
recipient-bound capabilities. Trusted hardware/hypervisor entropy and kernel
memory secrecy remain prerequisites; the conditioner does not claim recovery
from state disclosure. Restricted SHM, VM borrower exclusion, active revocation
and post-revocation native faults remain unimplemented.

The VFS mint syscall retains the full kernel PID through its trusted-role comparison. A host regression rejects `2^32 + CAPABILITY_BROKER_PID`; a low-word match does not authorize minting. This is an internal Rust API change from u32 to usize, with no serialized syscall change.
