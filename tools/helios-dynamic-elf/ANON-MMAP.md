# Helios anonymous mapping probe

Helios x86-64 tasks use the compatibility dispatch, verified in
`compat-linux/src/lib.rs` and `kernel/src/arch/x86_64/syscall.rs`.
`rax` selects mmap (9, translated through internal 1010 to the mmap
handler), mprotect (10 → 52), or munmap (11 → 51). Native mmap is 50.

| Call | Register arguments | Success |
| --- | --- | --- |
| mmap | rdi address, rsi length, rdx protection, r10 flags, r8 descriptor, r9 byte offset | guest virtual address |
| mprotect | rdi address, rsi length, rdx protection | zero |
| munmap | rdi address, rsi length | zero |

Addresses, lengths and offsets are 64-bit. The common handlers take
32-bit protection and signed 32-bit descriptors; the Helios mmap
translator validates the 64-bit Linux flag word before passing native
flags. Errors are negative errno in rax for Helios, with the precise
mapping in the syscall wrappers and `kernel/src/process/mmap.rs`.
Native wrappers return all ones. Invalid lengths and anonymous descriptor
arguments yield EINVAL; mprotect over an unmapped page yields ENOMEM.

The probe contract is one process with its initial thread, one 4096-byte
mapping, address zero, flags 0x22 (PRIVATE | ANONYMOUS), descriptor -1
and offset zero. The base returned is page aligned. The existing mapper
rounds nonzero lengths up with checked arithmetic and rejects nonzero
hints for nonfixed anonymous mappings. Protect and unmap require aligned
starts and use checked rounded ranges. Aligned zero-length mprotect with
a valid protection is an existing no-op.

The address-space owner owns the allocation, region metadata and page
tables. RegionProtection records R/W/X, with hardware user, writable
and NX bits implementing it. R and RW are supported, as are NONE and RX
in the underlying mapper. W-only, X-only, unknown protection bits and
write-execute requests are rejected.

File-backed, shared, partial and fixed mappings, inheritance, other
threads and concurrent access are outside this probe. The existing
shared descriptor path and anonymous partial range support remain in
place. Helios already has demand-backed anonymous pages; the probe
touches the page before checking protection and makes no overcommit claim.

Argument validation precedes mapping publication; the mapper advances
the nonfixed cursor only after commit. The compatibility wrapper may
initialize the cursor from the heap before calling the mapper. mprotect
preflights full region coverage and page state before mutation. munmap
preflights the mapped portion and permits holes; a repeated aligned
unmap is a successful no-op. The probe checks that the old address has
neither a region nor a leaf after unmap and that mprotect returns ENOMEM
there. It does not dereference the stale address.

## Positive path and live teardown

Markers 1–3 exercise the original probe, including explicit full unmap,
a stale protection request returning ENOMEM, and a repeated munmap
returning zero. Markers 4, 5 and 7 run as independent children and exit
with both pages of the primary mapping still live. Each reads a zero at
both ends before writing its marker. Two live pages add two reclaimed
frames and two intermediate tables relative to the explicit-unmap
control. Teardown reports zero remaining regions and owned frames, a
reclaimed page-table root, and a released kernel stack.

The orphan lifecycle control runs the same live probe with marker 8.
The child exits with status zero while its wait result remains pending.
The parent exits before collection. The unrelated observer gets
`Err(Failed)` from waitpid both before and after slot reuse. The
sequential mapping children independently verify fresh zeroed memory and
`Ok(0)` from their own parent. Same numeric PID recycling is
**not exercisable under current allocator contract**.

## Length and alignment

The guest requests 4096, 8192, and 4097 bytes. The last request receives
an 8192-byte region, verified by the ledger span and accesses at the
first and last bytes. With an aligned base, 4097-byte mprotect and
munmap requests also cover both pages. The scratch mapping uses a
separate read-only region, and protecting its second page after full
unmap returns ENOMEM. Zero-length and overflowing mmap requests return
EINVAL. Unaligned starting addresses and overflowing range lengths
return EINVAL for mprotect and munmap.

## Exhaustion

The probe alternates R and RW one-page regions to prevent ledger merging,
then touches each page. The ledger accepts 252 new regions and reaches
its 256-entry limit. The next mmap returns ENOMEM with the same free
frame count and region count. Protecting the rejected range returns
ENOMEM, and all 252 existing pages retain their contents. Exit reclaims
767 user frames and ten page tables, with zero residual regions and
owned frames. The next child reuses the slot and sees zeroed memory.

## Rollback coverage

| Case | Evidence |
| --- | --- |
| Frame allocation after validation | Existing MM-2A eager rollback test fails the second frame after installing the first page and restores the free-frame baseline and cursor. |
| Page-table allocation | Existing MM-2A tests fail the first lower table allocation and allocations after one and two intermediate tables. |
| Publication | The debug boot fixture cancels the eager anonymous reservation after leaf installation. Commit rejects the stale reservation and the isolated fixture checks the original region, cursor, free-page and owned-frame counts, then reclaims the root. |
| PTE update during mprotect | The focused debug fixture injects a recoverable rejection at the second leaf and restores the first. |
| Setup failure after mapping | Existing ELF startup injections cover initial stack, auxv and post-relocation allocation failure. The focused runtime fixture covers scratch setup failure with a primary anonymous mapping still live. |
| Interpreter/application setup | Existing handoff mapping and setup fault cases verify free frames, page-table counts, owned frames and absence from the scheduler. |
| Teardown failures | Address-space reclamation returns teardown counts. There is no recoverable teardown error path exercised by this gate. |

## Regression runs

Reproduce from the repository root with the local toolchain and an
already built local Limine:

```sh
export CARGO_NET_OFFLINE=true GIT_ALLOW_PROTOCOL=file
bash tools/helios-dynamic-elf/build.sh mmap complete/final2/local
python3 tools/helios-dynamic-elf/replay.py tcg --mmap complete/final2/local
python3 tools/helios-dynamic-elf/replay.py tcg --mmap complete/final2/local --replay
python3 tools/helios-dynamic-elf/replay.py kvm --mmap complete/final2/local
```

The harness verifies fixture and image hashes before each boot, uses two
vCPUs and disables the guest NIC. The repaired MM-2A host kernel suite
is reported separately in `host-mm2a.log`. Its Process fixture models
the production VFS admission and boot-root flags as booleans initialized
to false. Wayland and transport were not rerun. Qt and Qalculate! were
not launched.

Evidence is under `target/helios-runtime-mmap/complete/final2/local/`. The ISO
SHA-256 is
`364de5f6134a91b0a8b55b077832427b52aca1494bdb2056d16b343f0b092c4d`.

| Boot | Guest controls | Host ELF | PID allocator | Host kernel |
| --- | --- | --- | --- | --- |
| Initial two-vCPU TCG | Passed | 14 passed, 0 failed | 2 passed, 0 failed | 212 passed, 0 failed |
| Same-image two-vCPU TCG replay | Passed | 14 passed, 0 failed | 2 passed, 0 failed | 212 passed, 0 failed |
| KVM | NOT RUN — /dev/kvm unavailable | Unrun | Unrun | Unrun |

Both TCG runs passed the explicit-unmap, sequential-child, live-exit,
length, alignment, negative syscall, region exhaustion, eager allocation
and eager publication rollback controls, plus both focused runtime
fault cases. They also passed the existing
static and dynamic ELF, initialized and BSS-tail TLS, shared-object
local-dynamic TLS, direct TLS resolver rejection, TLS and dependency graph
rollback, and lifecycle controls. The offline mmap harness uses serial
output so it can run in workspaces without a QMP socket. The guest wait
limits remain the existing limits.

## Focused runtime failures

The normal mprotect path preflights full range coverage, leaf identity
and policy, stages the ledger change, updates the leaves with TLB
invalidation, and commits the ledger. Recoverable preflight errors
leave the range unchanged. An unexpected leaf mismatch after preflight
is a fatal invariant violation. That assertion remains in effect.

The debug-only marker P fixture requests RW → R across two populated
pages. The injector rejects the second update after the first leaf is
read-only. It restores the first leaf to RW, invalidates its TLB entry,
checks the frame identities and permission bits of both leaves, and
checks the original ledger against the page tables. The staged ledger
is discarded and the syscall returns `ENOMEM` (`-12`). The child
verifies both markers, writes both pages again, unmaps the whole range,
and obtains `ENOMEM` when protecting that range again. The parent
collects status zero.

The marker S fixture first publishes and populates an 8192-byte RW
mapping. Its later scratch setup requests a 4097-byte read-only mapping.
The injector cancels that reservation just before commit using the same
stale-reservation mechanism as the eager publication control. The
existing mmap error translation returns `EINVAL` (`-22`). The child
verifies both original markers and the absence of the scratch range,
then exits with fixture status 42, leaving the primary mapping live.
The parent consumes status 42 through waitpid. Marker Z is a different
child in the same slot and reads zero from both pages of a fresh
mapping. The fixture exercises setup after normal process publication
through normal exit and reclamation.

On both boots the P child reclaimed 515 frames and eight page tables,
matching the explicit-unmap baseline. Each S and Z child reclaimed
517 frames and ten page tables. Reclamation left zero owned frames and
regions, released the kernel stack, and reclaimed the root in all
three cases. The parent consumed each result, and S and Z occupied the
same slot with different process identities. See `result.json` and
`anon-mmap.log` in each accelerator evidence directory.
