# BTop-22: VirtIO completion validation before logical retirement

Base HEAD `540c43453af5e01df9e5adb251f0a6857a074a12`. Persistent
changes: `sunlight-virtio/src/blk.rs` and this report. The diagnostic
image uses the existing BTop-20 observer and trace build; temporary
patches were reversed. BTop-20 and BTop-21 evidence remains intact.

The first premature mutation was `pending = None` in `poll_transfer`:
it removed the software association from the polling path before reading
the used element. Ending the callback then announced completion;
`mapping.release()` dropped the user and `mapping.retire()` marked
the record retired. The used cursor was still unchanged. Neither that
cursor nor the logical mapping controls freeing the DMA pages, which
remain allocated for the attachment lifetime.

The moved block runs after `Owner::complete` and `Owner::retire` accept
the request. The exact pending token and active owner must match, and
the attachment must admit polling. Under the split-ring publication
ordering, the driver requires precisely one new entry (wrapping u16),
the expected chain head, a defined block status (0, 1 or 2), running
legacy transport, the published descriptor resources, and a submitted
owner for the current device and queue incarnation. The boot adapter's
exclusive borrow and the retained adapter's `VIRTIO_BLK` lock cover
validation and retirement; no other software consumer can substitute
a mapping between them.

Contract: OASIS VirtIO 1.4 CS01, pinned HTML SHA-256
`a48dd4967dda13cf2cf698f59b41ba1c4da0a428a30e9dfcd90ff2af3fc5d1c8`.
See §§2.7.8 and 2.7.13 for used-ring semantics, §4.1.4.10 for legacy
PCI, and §5.2.6 for block status. Used length remains diagnostic under
§5.2.6.3; it grants neither short-read success nor a partial prefix.
A validated IOERR or UNSUPP ends ownership without copying or crediting
successful bytes. The callback's Completed state records the end of its
lifetime, independently of I/O success.

An invalid record returns Queue/Uncertain with unknown completed bytes,
no caller copy and no success credit. The existing quarantine stops
admission and polling, leaving the pending token, active callback and
one authorized mapping user. The slot cannot drain to completion on
this evidence: the record does not establish that the device is done
with this chain. There is no completion worker or authorized live reset
path here to resolve that ownership. The storage ledger's call-return
cleanup remains separate from driver ownership.

| Host gate | Result |
| --- | --- |
| Logical retirement ordering | **PASS**. Same BTop-21 injection: delta 1, head 1 instead of 0, length 513, status 0. Original witness: consumed 0, callback Completed, mapping retired. Updated retention assertion on old production code fails at pending (None). Fixed code: consumed 0, callback Active, pending retained, mapping live with one user, no output copy. |
| Validated completion | **PASS**. Exactly one consume and retire; polling again or timing out the old token cannot affect the next live mapping, even when the descriptor head is reused. Existing validated error and malformed index/status/descriptor tests passed. |
| Stale wire observations | Old software token and an unchanged already consumed index are covered. A newly published wire-identical duplicate cannot be distinguished from a conforming new completion because the wire head carries no generation; the existing T17 witness preserves this limit. |
| Timeout/quarantine regression | **PASS**, host only. In both deadline-winning orders late polling leaves the Deadline receipt and caller buffer intact; a fake zero reset acknowledgement retains the mapping. Terminal retirement after timeout **BLOCKED**; guest timeout/reset **NOT RUN**. |
| Physical DMA reclamation | **BLOCKED** for uncertain ownership. Host quarantine prevents reuse; no physical DMA reclamation or live fence was demonstrated. |
| Focused host suite | **PASS**, 255 tests. |

## Final guest acceptance

Both replay processes exited 0 on the final ISO under QEMU TCG with
4 GiB and four vCPUs. The mounted QEMU wrapper replaces the snapshot
argument with `readonly=on`; QMP independently confirmed the fixture
was read-only and reported zero write bytes.

| Checkpoint | Completions / sectors |
| --- | ---: |
| Child registration | 0 / 0 |
| Mount and before idle | 1,011 / 1,011 |
| After idle | 1,011 / 1,011 |
| First payload | 1,158 / 1,158 |
| Repeat payload | 1,304 / 1,304 |

The lifecycle verifier counted two boot and 1,302 retained requests.
The BTop-16 verifier reconciled the snapshots with QEMU request and
completion LBA multisets and QMP byte deltas. It excluded 814
pre-registration sectors (15 touching LBA 0 and 128 within the future
child). Each payload verified 65,536 bytes and EOF: 128 payload
sectors plus 19 and 18 other sectors, respectively. CD I/O was separate.
The same-vCPU diagnostic intervals totaled 131,317,720 ns; production
timing was unchanged.

| Guest gate | Result |
| --- | --- |
| Mounted acceptance | **PASS**. Help and return in `help.png` and `dashboard-return-0.png`; explicit q, worker 44 and leader 42 reaped (serial lines 8345, 8357), btop exit 0 (8361), separate same-TTY proc-native exit 0 (8389). |
| CD-only acceptance | **PASS**. Both probes returned ENOENT and exit 3 with no fixture registration or attributed requests. Help and return in the same named captures; explicit q, worker 41 and leader 40 reaped (3012, 3024), btop exit 0 (3028), separate same-TTY proc-native exit 0 (3055). |
| Live guest invalid completion, timeout/reset, parity | **NOT RUN**. |
| BTop-12 / BTop-13 discovery | Historical **FAIL** / **BLOCKED**, respectively. |

Raw evidence is in `target/btop-22/{mounted,cd}`: serial and QEMU
logs, QMP input and scanout records, phase offsets, attachment and
block stats JSON, and interaction captures.

## Commands and artifacts

```sh
out=target/btop-22
mkdir -p "$out"
# Before editing:
cargo test --target x86_64-unknown-linux-gnu -p sunlight-virtio btop21 -- --nocapture > "$out/before.log" 2>&1
git show HEAD:sunlight-virtio/src/blk.rs > "$out/blk.before.rs"
# After editing, temporarily revert only the two production hunks:
git diff -- sunlight-virtio/src/blk.rs | awk '/^@@/ && ++h == 3 {exit} {print}' > "$out/production.patch"
(
    git apply -R "$out/production.patch" || exit
    trap 'git apply "$out/production.patch"' EXIT
    cargo test --target x86_64-unknown-linux-gnu -p sunlight-virtio btop21_invalid_completion -- --nocapture > "$out/before-assertion.log" 2>&1
    test "$?" -eq 101
)
cargo test --target x86_64-unknown-linux-gnu -p sunlight-virtio btop21 -- --nocapture > "$out/after.log" 2>&1
cargo test --target x86_64-unknown-linux-gnu -p sunlight-block -p sunlight-virtio -p sunlight-fat -p sunlight-fs -p sunlight-compat-linux > "$out/focused.log" 2>&1
sed 's@target/btop-14/final@target/btop-22/final@g' tools/btop-14/image.sh > "$out/image.sh"
(
    set -e
    git apply tools/btop-20/observe.patch
    trap 'git apply -R tools/btop-20/observe.patch' EXIT
    SUNLIGHT_BTOP16_TRACE=1 bash "$out/image.sh"
) > "$out/image.log" 2>&1
sed 's@target/btop-21/replay.generated.py@target/btop-22/replay.generated.py@g' target/btop-21/replay.py > "$out/replay.py"
cp target/btop-20/payload-lbas.txt "$out/"
iso=$out/final/guest-ramfs-trace-config.iso
python3 "$out/replay.py" "$iso" "$out/mounted" --disk target/btop-10/disk.img > "$out/mounted.log" 2>&1
python3 "$out/replay.py" "$iso" "$out/cd" > "$out/cd.log" 2>&1
xorriso -osirrox on -indev "$iso" -extract /boot/sunlight-kernel.elf "$out/iso-kernel.elf" > "$out/extract.log" 2>&1
cmp "$out/iso-kernel.elf" "$out/final/kernel-ramfs-trace-config.elf"
python3 -c 'from pathlib import Path; import hashlib; b=Path("target/btop-01/btop").read_bytes(); assert hashlib.sha256(b).hexdigest()=="bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702"; assert b in Path("target/btop-22/iso-kernel.elf").read_bytes()'
sha256sum -c "$out/hashes.sha256"
for mode in mounted cd; do
    python3 tools/btop-20/lifecycle.py "$out/$mode" > "$out/reconstruct-$mode.log" 2>&1
    python3 tools/btop-16/verify.py "$out/$mode" > "$out/verify-$mode.log" 2>&1
done
git diff --check
```

| Artifact | SHA-256 |
| --- | --- |
| Final diagnostic ISO | e7575a42b56655c41b578157b8cf4fa9c2e575dd25b8bffff5d90a8bb79eb148 |
| Kernel ELF extracted from ISO | c3ff5bcb3454c779f57f4049c69835ccea34de93470e0d0b0f046ef39243856b |
| Complete btop binary verified inside extracted ELF | bee0ba888aa05952281522fc34a8733a4eb3521267e02e7e045f33fa4ba34702 |
| Storage probe ELF | 7a325dfdf08e493157bcf98ca2e035a8b71f95b8278960177b9f2a7c8bfd32fb |
| Read-only fixture | 4acf7c2944980eee673fb4d1c59af9add831f57b3b8bc26fd127fe55a2bc7d8f |
