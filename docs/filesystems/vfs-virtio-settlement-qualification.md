# VFS-N1H: Isolated VirtIO settlement

Ordinary settlement passed on one disposable QEMU legacy VirtIO block
device. This supports a serialized normal-path settlement adapter, but
does not qualify an N1D device attachment or ownership transfer.

| Boundary | Existing transport behavior |
| --- | --- |
| Initialization | Reset status must read zero before queue publication. Initialization negotiates legacy features including FLUSH, sets the queue PFN, allocates a boot-local incarnation and sets DRIVER_OK. Readiness in the probe also requires usable geometry and a successful marker read. |
| Submission | One active request owns the descriptor chain and one pinned bounce page. Writes copy data into the page before publishing and notifying. A staged write returns its request ID while ownership and one mapping user are still live. |
| Completion | The driver checks the used index, head, device and descriptor state and status before retiring ownership and releasing the mapping user. Reads copy out from the bounce page after validation. A successful data receipt accounts for exactly 512 bytes; valid non-success status does not imply a successful write. The wire head does not echo the software generation. |
| Drain and flush | Drain tracks the operation slot; the probe also requires no outstanding request and a retired mapping with zero users. Negotiated FLUSH success certifies a virtual barrier through the preceding queue generation. Ordinary write success does not establish durability. |
| Timeout and shutdown | Timeout or malformed completion quarantines the device and can retain the live request and DMA. Terminal reset closes admission; even acknowledged reset remains `VerifiedRetained`. Dropping the handle gives no DMA fence. Neither descriptor reuse nor logical mapping retirement frees queue or bounce frames. |

The feature-gated probe runs before ordinary disk registration and FAT
admission. It holds the device exclusively and checks capacity, writable
geometry, FLUSH support and the fixture signature before writing. The
harness boots from a separate ISO and creates the sole attached VirtIO disk
in a fresh directory. Each of 64 cycles checks a submitted write with one
pending operation and one mapping user, exact request retirement and drain,
a confirmed flush covering that write, and an independent readback. The
guest checked 193 completions including the initial read. The harness then
compared all 4 MiB of the image with the expected bytes, including untouched
sectors. This does not measure persistence across host power loss.

This probe exercises the transport side of N1E–N1G: exclusive ownership at
the test boundary, one serialized data slot, correlation by device and queue
incarnation and request generation, and a barrier after drain. It does not
run the host lease fixture against hardware; that module is excluded on
`target_os = "none"` and owns a fake image. Each cycle reuses the same
attachment and DMA allocation. There were no lease transfers, device
reinits, asynchronous caller withdrawals or physical frame releases.

Before N1D can consume these receipts, bind the kernel admission authority
to canonical device aliases, authenticated registration, medium and lease
generations, extent and rights. Reserve actual transport capacity (one
active request on this queue); separately account for caller credits and
retained buffers, and correlate each accepted ticket with its exact
transport request. Close admission before drain, finish predecessor cache
writeback and require a barrier after it. Qualify failure paths for lost
callers, ambiguous effects, late completion, reset and removal; retain DMA
until an authoritative fence permits reclamation. Guest fault handling,
physical hardware, and DMA frame release were not tested.

After a proven handoff, a fresh read-only backend must verify geometry,
media and root identity. Pin readiness through N1D's exact reservation and
topology checks; preserve caller and FD installation fences, rollback and
retained-reference withdrawal. Until then, leave production routing, mount
publication and real FAT ownership transfer inactive.

Reproduce with an existing `target/sunlightos.iso` and local Limine tools:

```sh
CARGO_INCREMENTAL=0 cargo build -p sunlight-kernel --features vfs-n1h-test --offline
python3 tools/test_vfs_n1h.py
```

See [validation results](vfs-virtio-settlement-qualification-results.json).
