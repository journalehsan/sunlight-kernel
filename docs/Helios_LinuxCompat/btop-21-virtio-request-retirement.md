# BTop-21: VirtIO request retirement

Validated used-ring completion retires a Helios read once. After timeout the
current read path retains the request and never consumes its late completion.
Terminal accounting for that outcome is **BLOCKED**. BTop-20's 1,304 reads
and 116.269 ms candidate remain historical evidence. BTop-12 remains **FAIL**
and BTop-13 discovery remains **BLOCKED**.

## Transport and ownership

Pinned: OASIS VirtIO 1.4 Committee Specification 01, HTML SHA-256
`a48dd4967dda13cf2cf698f59b41ba1c4da0a428a30e9dfcd90ff2af3fc5d1c8`.
See §2.4.1–2.4.2 for reset, §2.7.6–2.7.8 and §2.7.13 for split-ring
publication and consumption, §4.1.4.10 for legacy PCI and §5.2.6 for block
requests. `pci.rs::find_virtio_blk` requires an I/O BAR, and `blk.rs::init`
uses legacy port I/O, queue 0 PFN, split rings, and no queue-reset feature.
It waits for zero status before configuring the queue. The kernel allocates
four queue pages and one request page in `main.rs` and passes direct-map
addresses; these remain allocated for the attachment lifetime.

| Transition | Ownership and synchronization |
| --- | --- |
| Admission and publication | The boot adapter borrows the driver exclusively; the retained adapter holds `VIRTIO_BLK` through the synchronous call. `start_transfer` obtains a software token (device/queue incarnation, generation, head) and prepares head 0, header, 512-byte device-write data region and status initialized to a sentinel. The driver retains the token; volatile writes and SeqCst fences order descriptor/avail-slot setup before avail-index publication and the queue kick. From index publication the device may use the chain and bounce memory. The caller's output buffer is never a DMA target. Pending and mapping bookkeeping are set before any software consumer can poll, under the exclusive driver borrow. |
| Observed completion | `transfer_observed` polls volatile used index; `poll_transfer` checks the active token and `finish_transfer` orders used-index observation before consuming the entry/status. It requires the exact next index, matching head, valid status, running transport, matching descriptor resources and owner completion. It then advances the consumed index and calls `Owner::retire`. A conforming device has finished using the returned chain; the exclusive driver can reuse chain and bounce page on a later request. Pages are not returned to PMM. Success copies 512 bytes before return; validated device error retires without copying or crediting successful sectors. Used length is diagnostic. |
| Deadline | After the final empty sample, `timeout_transfer` quarantines, requests slot cancellation and returns typed Deadline/Uncertain (legacy error). The token stays pending, the mapping retains one user and the device may still own the chain and bounce page. A used entry can land in the final sample-to-timeout gap. There is no final recheck, and the deadline still wins that order. |
| Late used entry | Neither a completion worker nor a read IRQ handler claims it. `poll_transfer` refuses quarantine before touching the ring; subsequent admission refuses before rewriting it. The entry can exist in memory without becoming an observed terminal event. No reuse or second caller result follows. |
| Reset and teardown | `begin_terminal_reset` closes admission, quarantines and writes zero once to status at I/O offset 0x12. `poll_terminal_reset` reads status at most 64 times. Zero readback is the spec's completion signal: a conforming device must cease queue interaction and notifications until reinitialization. Nonzero running status is still pending; budget exhaustion or unexpected status remains terminally retained. `VerifiedRetained` does not consume the used ring, retire the request, free frames, reopen admission, or resolve the caller result. The SFS coordinator and reset diagnostics are separate from the Helios read timeout path. Dropping the handle is not a DMA fence; there is no reclamation teardown. |

`storage::read_virtio` calls `DeviceRegistry::retire_read` after a
published call returns, including a deadline, and `ReadBusy` also ends
at return. Those are existing ledger operations, not proof of terminal
ownership. No production timing or read path changed.

## Deterministic host evidence

The tests exercise real private driver transitions on fake pages, with
ordered interleavings rather than a racing host thread. Same-head reuse
after validated completion rejects the old software token and does not
consume the old used entry again. The wire ID carries a head, not a
generation. This proof assumes compliant used-ring publication; see the
preserved SFS-5B.2 T17 witness for the wire-identical stale-entry limit.
After timeout there is no newer request on the quarantined attachment,
including after simulated zero readback.

```text
invalid head: Queue/Uncertain consumed=0 callback=Completed mapping_retired=true
poll-first: consumed=1 mapping_retired=true old_token_rejected=true
used-before-timeout: Deadline consumed=0 users=1 reset=VerifiedRetained mapping_retired=false
used-after-timeout: Deadline consumed=0 users=1 reset=VerifiedRetained mapping_retired=false
```

Separate bookkeeping defect: `poll_transfer` clears pending, finishes
the callback and releases/retires the logical mapping before
`finish_transfer` validates the entry. A malformed head reports Queue /
Uncertain with no consumed index and no caller copy, yet the callback
reports Completed and the mapping reports retired. The host reproducer
preserves this discrepancy. It does not establish physical free or reuse:
the owner and uncertain latch prevent subsequent admission. The earlier
physical reuse failure is preserved under
`docs/audit/sunfs/sfs-5a/raw/ownership-before.log`; current timeout
quarantine prevents that reproducer's reuse. Do not treat mapping state
or callback drain alone as authoritative evidence for accounting or DMA reuse.

## Guest replay and results

Reused the byte-identical BTop-20 ISO; no new diagnostic ISO. No controlled
delay qualified for the pinned read-only replay, so no guest fault or
live timeout was induced. Mounted counts from zero at registration:
1,011 at mount, unchanged through idle, 1,158 after first payload, and
1,304 after repeat, with sectors equal to completions. Independent
events show 2 boot and 1,302 retained requests. QEMU LBA and QMP byte
verification excludes 814 pre-registration sectors, 128 within the
future child and 15 touching LBA 0; CD traffic belongs to another
transport. Each payload accounts for 128 sectors, with 19 and 18
additional sectors respectively. Fixture writes were zero. Candidate
total was 118,032,792 ns, a new observation on the same image, with
same-vCPU endpoints per request (not a subtraction across vCPUs).

| Gate | Result |
| --- | --- |
| Normal completion | **PASS**: fake-ring validate/consume/retire and normal mounted read-only counts. |
| Timeout and late completion | **PASS** for host containment and caller status in both deadline-winning orders; terminal event **BLOCKED**. Guest timeout **NOT RUN**. |
| Reset | **PASS** for host simulated acknowledgement retaining resources, with existing nonzero/exhaustion tests; live reset **NOT RUN** here. Reclamation or terminal accounting after reset **BLOCKED**. |
| Exactly once | **PASS** for validated completion and software token rejection; timeout terminal accounting **BLOCKED**. Callback/mapping validity on malformed completion **FAIL**, separately reproduced. |
| Focused regression | **PASS**, 255 host tests. |
| Mounted interaction | **PASS**: Help open/return captured in `help.png` and `dashboard-return-0.png`; explicit q, worker 44 and leader 42 reaped (serial lines 8359 and 8371), btop exit 0 (8375), separate same-TTY proc-native exit 0 (8400). |
| CD-only acceptance | **NOT RUN** on this replay; BTop-20 acceptance retained on identical ISO. |
| Cross-vCPU timing | **NOT RUN**; no migration/offset qualification. |
| Same-image native/musl parity | **NOT RUN**. |
| Linux read-request time | **BLOCKED** for production equivalence. |
| Linux `io_ticks`, stats export and btop I/O display | **NOT RUN**, separate gates. Writable guest I/O and hotplug out of scope. |

## Reproduction

Base HEAD `ca178af0ce904e6f4031ccb3499646dfe5a11ca9` plus incoming dirty
sources. Changed paths: this report and host tests in
`sunlight-virtio/src/blk.rs`. No commit or push.

```sh
out=target/btop-21
mkdir -p "$out"
curl -fsSL https://docs.oasis-open.org/virtio/virtio/v1.4/cs01/virtio-v1.4-cs01.html -o "$out/virtio-v1.4-cs01.html"
sha256sum "$out/virtio-v1.4-cs01.html"
cargo test --target x86_64-unknown-linux-gnu -p sunlight-virtio btop21 -- --nocapture >"$out/host.log" 2>&1
cargo test --target x86_64-unknown-linux-gnu -p sunlight-block -p sunlight-virtio -p sunlight-fat -p sunlight-fs -p sunlight-compat-linux >"$out/focused.log" 2>&1
sha256sum -c target/btop-20/hashes.sha256 >"$out/hashes.log"
sed 's@target/btop-20/replay.generated.py@target/btop-21/replay.generated.py@g' target/btop-20/replay.py > "$out/replay.py"
python3 "$out/replay.py" target/btop-20/final/guest-ramfs-trace-config.iso "$out/mounted" --disk target/btop-10/disk.img >"$out/mounted.log" 2>&1
cp target/btop-20/payload-lbas.txt "$out/"
python3 tools/btop-20/lifecycle.py "$out/mounted"
python3 tools/btop-16/verify.py "$out/mounted"
git diff --check
```

| Artifact | SHA-256 |
| --- | --- |
| ISO | e94c8df4f240db0a0af5ee75a40962a6a0722c3bc44b295943c10a203b78b4b0 |
| Kernel ELF | d6e90cfae72b5781225a099c2773ef69fdf407d68028f174d5255b54154a0eba |
| btop | bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 |
| Read-only fixture | 4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f |

Raw replay evidence: `target/btop-21/mounted/{command.json,serial.log,qemu.log,phases.jsonl,attached.json,block-*.json,qmp-input-scanout.jsonl}`.
