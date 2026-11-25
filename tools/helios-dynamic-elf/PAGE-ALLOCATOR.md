# Helios guest page-region allocator

The freestanding C implementation in [page-allocator.h](page-allocator.h) runs inside
one Helios x86-64 guest process on its initial thread. It uses a zero-initialized
table with the process's guest PID as its owner. Initialize it once per process
and keep it for the lifetime of that process. The allocator provides page regions.
It is not a malloc/free ABI or libc support.

| Operation | Result |
| --- | --- |
| `page_alloc(allocator, backend, pages, &handle)` | Returns zero and writes a complete handle on success. Leaves the output untouched on error. |
| `page_resolve(allocator, handle)` | Returns the guest virtual base of the live region, or zero for an invalid handle. |
| `page_free(allocator, backend, handle)` | Unmaps the exact original base and size. Returns zero and invalidates the handle on success. |

The allocation unit and base alignment are 4096 bytes, from the existing x86-64
`Size4KiB` mmap ABI. Callers supply integral page counts. There is no rounding or
other alignment option. The table holds at most eight live allocations of at
most eight pages each, with a combined limit of 16 live pages. Pages read as zero
before the first write under the anonymous private mmap ABI. The allocator uses
RW, flags 0x22, descriptor −1, offset zero and a zero address hint. The mapper
retains its existing lazy zero-fill behavior.

A handle contains the guest process owner, monotonically increasing allocation
generation, table slot, guest mapping base and page count. It contains no host
pointer. Ownership comes from the live table entry. Copying a handle permits access
to the same entry until the first successful free. A copied handle does not add
an owner. The generation never wraps and remains distinct when a table slot is
reused. Handles are validity tokens for cooperating code in one process. The API
excludes transfers, inheritance, concurrent or reentrant calls, partial free and
reallocation. Callers must leave the backing mappings under allocator control.

- `-EINVAL` (`-22`) rejects zero pages, overflow, requests above eight pages,
  and stale, duplicate, foreign, interior or otherwise mismatched handles.
- `-ENOMEM` (`-12`) reports a full metadata table, page budget or generation
  exhaustion. Backing syscall errors are propagated unchanged.
- `-EIO` (`-5`) reports rejection by the fixture-only metadata publication
  gate after the backing mapping has been undone.

A successful free allows table-slot reuse. The current hintless mmap path
advances its virtual cursor, so a replacement can have a new guest virtual
base. The host metadata suite also tests exact address reuse. Only a live handle
can resolve an allocation. Previously cached guest addresses have no allocator
validity guarantee.

The allocator validates limits and selects a vacant slot before mapping. The
entry becomes visible only after the backing mapping and metadata publication
succeed. If the fixture rejects publication, it releases the whole mapping
before returning an error. If that rollback itself fails, the backend terminates
the guest and the process teardown path reclaims the mapping. An ordinary
recoverable `munmap` failure leaves the handle and all accounting live. The
kernel owns mapping ledger entries, page frames and page tables. Process exit
discards the guest table with the rest of its address space and reclaims all
remaining mappings, including when the parent exits before collecting the child.

## Regression fixtures

The allocator probe is a freestanding mode of the existing mmap ELF, baked into
the allocator regression image. The build records the compiler command and
stamped ELF hash. The gate checks the manifest, interpreter, guest ELFs,
launcher, kernel and ISO hashes before every available boot. It uses two vCPUs,
the existing wait limits, and a disabled guest NIC.

- `A` verifies zero-filled mappings, non-overlap, full marker readback, exact
  free, handle rejection, and independent table and page limits.
- `F`, `T`, and `L` exercise a second-frame failure, lower page-table allocation
  failure, and stale mapping publication reservation after three staged leaves.
  These debug-only cases route the selected three-page requests through the
  existing eager mapper so it can return the errors synchronously. They verify
  the free frame count, per-process owned frame count, region count and cursor
  immediately after rollback.
- `Q` rejects guest metadata publication after touching pages in the new mmap.
  It verifies that rollback releases those pages and preserves the existing owner.
- `E` fills the actual mapping ledger and verifies mmap returns `ENOMEM`.
  Freeing the middle of a merged region also returns `ENOMEM` while the ledger
  is full. The probe verifies all three markers, releases a scratch entry and
  verifies that free and a subsequent allocation succeed.
- `a` through `g` leave six populated allocator pages live at exit. The harness
  checks fresh metadata and distinct per-child markers, sequential child slot
  reuse, uncollected exit, unrelated wait rejection and replacement children.
  Resource evidence records frame and page-table counts, reclaimed roots,
  released stacks and zero remaining regions or owned frames.

Same numeric PID recycling remains **not exercisable under the current
allocator contract**. Wayland and transport were not rerun. Qt and Qalculate!
were not launched.

## Run locally

```sh
export CARGO_NET_OFFLINE=true GIT_ALLOW_PROTOCOL=file
bash tools/helios-dynamic-elf/build.sh allocator local
python3 tools/helios-dynamic-elf/replay.py tcg --allocator local
python3 tools/helios-dynamic-elf/replay.py tcg --allocator local --replay
python3 tools/helios-dynamic-elf/replay.py kvm --allocator local
```

Evidence is written to `target/helios-runtime-allocator/local/`. The per-boot
`result.json` correlates the guest mapping identities and ownership transitions
with lifecycle, waitpid and reclamation records. `page-allocator.log` records
the allocations, frees, injected failures and backing map/unmap transitions.
`host-page-allocator.log` contains the focused metadata tests.

## Verified matrix

Both TCG boots used ISO SHA-256
`0a54b5282c62644730a2dbd9f7914273640c0d9e7da24cc3aa4f1b6c1971513e`.
The stamped mmap and allocator fixture ELF SHA-256 was
`935cb34d468c7819c2fd0610a2c794fc29aa1d75f46d42e1dbeb1a434d1c1719`.

| Run | Guest allocator and regression controls | Host kernel | Host ELF | PID allocator | Host page allocator |
| --- | --- | ---: | ---: | ---: | ---: |
| Initial two-vCPU TCG | PASS | 212/212 | 14/14 | 2/2 | 7/7 |
| Same-image two-vCPU TCG replay | PASS | 212/212 | 14/14 | 2/2 | 7/7 |
| KVM | NOT RUN — /dev/kvm unavailable | Unrun | Unrun | Unrun | Unrun |

The guest controls included anonymous mmap/mprotect/munmap, ELF loading and
rollback, initialized and BSS-tail application TLS, shared-object local-dynamic
TLS, direct resolver rejection, TLS and graph rollback, lifecycle and waitpid.
Each live-exit allocator child reclaimed 523 frames and 10 page tables,
compared with 517 frames and eight page tables after explicit free. Teardown
left no owned frames or mapping regions and reclaimed the page-table root.
