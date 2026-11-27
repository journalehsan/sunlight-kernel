# VFS-N1G: Transport settlement and DMA lifetime

## Current storage paths

| Path | Ownership, settlement and limits |
| --- | --- |
| Kernel disk routing | [KernelBlkDev](../../kernel/src/main.rs) serializes through the retained virtio or USB device mutex. Partition views restrict addresses; they do not own a distinct hardware queue. Virtio FAT uses a writeback cache; USB bypasses it. [CachedBlockDevice::flush](../../sunlight-block/src/lib.rs) writes dirty slots before requesting the underlying barrier. A dirty bit cleared by write completion does not certify persistence. The legacy BlockDevice result does not expose transport lifetime. |
| Virtio submission and completion | [VirtioBlk](../../sunlight-virtio/src/blk.rs) owns one active request, the descriptor chain, queue pages and request bounce page. Driver submission publishes descriptors before notifying; the driver polls or, on the staged write path, explicitly advances completion. It validates used index, head, status, descriptor identity and running state before retiring request ownership, the operation slot and mapping user. The completion can allow reuse of the fixed buffers; it does not free their allocation. The legacy wire head does not echo a request generation. |
| Virtio failures and flush | A timeout or invalid completion quarantines the attachment and may return uncertainty while the request and DMA pages remain live. Software observer retirement is separate. Terminal reset closes admission and polls for acknowledgement; even verified reset is recorded as retained and does not certify earlier writes or free DMA. Removal-like or unexpected status and missing acknowledgement also retain resources. Successful negotiated FLUSH gives a typed virtual barrier through preceding generations. Unsupported, failed or uncertain flush gives no such proof. |
| USB BOT/xHCI | [BOT](../../sunlight-block/src/usb_msc.rs) owns the tagged command through CBW, data and validated CSW; [xHCI](../../kernel/src/usb_xhci.rs) owns TRBs, ring state and the pinned bounce buffer, matching transfers by endpoint and TRB pointer. Caller slices are copied through that buffer. BOT recovery does not replay a possibly executed write; endpoint stop/dequeue and class recovery are transport recovery, not a durability receipt. Runtime readiness or I/O failure retires the disk incarnation. Port disconnect or a timeout is not a DMA release event. [USB initialization](../../kernel/src/usb_storage.rs) retains DMA on failure; there is no VFS handoff reclamation contract. Successful SCSI SYNCHRONIZE CACHE is the current barrier path. |
| Shared bookkeeping | [OpSlot and MappingRecord](../../sunlight-block/src/quiesce.rs) separate terminal callback state from mapping users and retirement. A cancellation request leaves the operation live. Bounded drain reports incomplete work; neither atomic bookkeeping nor a mapping user count independently proves hardware fencing or persistence. |

## Host settlement contract

The existing N1E/N1F authority remains the sole owner of the device slot.
Authenticated sessions, the extent and rights, and the whole-device lease
still govern admission. Every accepted data operation charges a caller credit
and reserves a transport slot containing one fixed block buffer. The ticket
identifies the device scope, lease and medium generations, and the unique
request serial. Both pools are bounded by four, independently; a full
transport can refuse new work after all caller credits have been returned.

The trusted coordinator may abandon a caller credit after cancellation or
timeout and keeps the ticket for late transport acknowledgement. Abandonment
neither aborts the command nor resolves its effect. A deterministic terminal
acknowledgement removes the request and releases its retained buffer exactly
once; it returns a still-charged caller credit in the same step. The returned
completion belongs to the coordinator, which must not deliver an abandoned
reply. A terminal acknowledgement with an ambiguous effect can reclaim the
buffer but latches a fence. Transport loss or media removal alone latches a
fence and keeps the request and buffer; late proof can reclaim them without
clearing that fence. Stale or duplicate tickets change neither accounting
nor a successor. If proof never arrives, the slot stays occupied.

Quiescing closes data admission before drain. A flush can start only after
the last transport slot is empty, so zero caller credits is insufficient.
The barrier has its own retained ticket; cancelling a transition with a
submitted barrier cannot reopen admission before it settles. Confirmed
success synchronizes the fake image and enables transfer or release on the
exact transition; authenticated transfer revalidates incumbent and successor.
Failure without a barrier proof never authorizes a successor, and uncertainty
keeps the device fenced. All operations are bounded steps under the existing
authority lock, with no waiting or callbacks. Buffer reclamation here means
removal of an owned host value; there are no physical mappings, controller
queues, reset proofs or hardware DMA in this fixture.

## Next integration milestone

Qualify one kernel-owned virtio transport on an isolated sacrificial device
with no live FAT mount. Use the typed completion path to record request
retirement, DMA reuse eligibility and a confirmed barrier separately. Bind
them to canonical device and queue incarnation, current medium, lease and
authenticated registration. Exercise normal handoff eligibility plus timeout,
late completion and reset/removal fencing. Keep pages pinned on uncertainty;
leave USB and general reset recovery out of this first integration. Do not
enable production routing to obtain the evidence.

Then use those observations in the [N1D attachment transaction](vfs-service-attachment-transaction.md).
Reserve an unpublished attachment. Freeze predecessor mutation and finish
backend writeback; close device admission, drain, confirm the barrier and
revalidate the settlement against the exact transfer. A successor can then
probe read-only through a fresh backend/cache. Bind readiness to the
reservation, canonical device, media and lease generations, registration,
extent and rights, geometry and root identity. Hold a bounded readiness pin
through N1D publication; revalidate target, topology and owner, and retain
the distinct caller-view and kernel FD installation checks. Never wait with
namespace, device admission or completion locks held.

Before transfer, cancel an untouched reservation once pending obligations
settle. After transfer, keep failed publication fenced, withdraw the
unpublished backend and drain it; regrant only with proven unchanged medium
and no staged writes, a fresh grant and fresh backend validation. On
withdrawal, freeze admission by exact mount incarnation, permit close and
writeback, and wait for retained roots and descriptions to retire before
reclaiming the backend. Preserve shared description offsets and explicit
PREAD offsets. If settlement or durability remains uncertain, require
explicit recovery; no timeout alone releases ownership.

## Evidence

Four deterministic tests prove host retention, delayed and out-of-order
acknowledgements, transfer gating, loss and removal fencing, and concurrent
slot accounting across 64 sequential lease lifecycles. The 477 baseline
tests are preserved by name; 481 pass including N1G. Both requested builds
pass. The unchanged ownership guard has identical output to N1F: 71
references and four unmatched live dependencies (statfs, ProcFS mountinfo,
private mmap and runtime FAT mount). All remain open. See
[machine-readable results](vfs-transport-dma-settlement-results.json).
