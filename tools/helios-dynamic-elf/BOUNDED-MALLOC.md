# Helios bounded fixture malloc

The freestanding x86-64 shim in [bounded-malloc.h](bounded-malloc.h) is a
fixture/runtime interface with its own checked handle API. It provides no libc
malloc ABI. Initialize one zeroed heap per process and set `heap.pages.owner`
to its guest PID. Keep that heap for the process lifetime and use it from the
initial thread. The backend is the existing anonymous page allocator backend.
Calls require valid heap, backend and output storage. The fixture hook must
return without reentering the heap.

| Operation | Result |
| --- | --- |
| `helios_malloc(heap, backend, size, publish, &allocation)` | Returns zero and writes a handle. On error the output is unchanged. Pass `0` for the fixture publication hook in ordinary use. |
| `helios_resolve(heap, allocation)` | Returns the live allocation base as a guest address, or zero for an invalid handle. |
| `helios_free(heap, backend, allocation)` | Returns zero and invalidates all copies of the handle after successful unmap. |

| Limit | Value |
| --- | ---: |
| Minimum request | 1 byte |
| Maximum request | 32,768 bytes |
| Base alignment | 4,096 bytes |
| Live allocations | 8 |
| Live backing | 16 pages, 65,536 bytes |

Each request takes its own mapping. Sizes from 1 through 4,096 bytes use one
page. Larger sizes round up to a whole number of pages with checked addition.
The total budget counts rounded backing bytes. The page allocator chooses the
first vacant slot. A free slot can be reused immediately with a fresh published
generation. The hintless guest mapper generally supplies a new address. If a
backend recycles an address, the old handle stays invalid. The anonymous
mmap backend provides zero-fill before first access across the whole backing
mapping, including reused storage. The shim delegates that guarantee to its
backend. Callers may use the requested byte extent.

The handle contains the requested size and a page ownership token with guest
owner, generation, slot, base, and backing page count. The live entry combines
the page allocator's token with the shim's nonzero requested size and
recorded alignment. The alignment has one supported value. The slot state distinguishes published
allocations from free and staged slots. Resolution checks the full token and
requested size against the live entry before accepting the aligned base. The
token is for cooperating guest code. Ownership comes from the heap of the
current process. Keep mappings under allocator control and discard any cached
address after free.

- `-22` (`EINVAL`) rejects size zero, oversize, rounding overflow and invalid
  handles. Invalid handles include an all-zero token, duplicate or stale
  tokens, foreign owners, generation or size mismatches, misaligned addresses,
  and aligned interior addresses. Rejection occurs before unmap.
- `-12` (`ENOMEM`) reports slot, backing budget or generation exhaustion.
  Backing syscall errors propagate unchanged.
- `-5` (`EIO`) reports a fixture publication failure after cleanup. The
  request acquires backing before recording its requested size and returning
  the public token. A hook rejection releases staged backing and restores
  allocator accounting. A rollback unmap error terminates the fixture so the
  kernel can reclaim the address space.
- A recoverable unmap error during an ordinary free leaves the allocation,
  contents, token and accounting live. Kernel process teardown reclaims live
  mappings and discards shim state on child exit, including uncollected exit.

## Verification

The malloc image includes the original mmap and page allocator controls and
repeats the allocator cases through the shim with byte-sized requests. The foreign
ownership case passes a real handle from a producer child to a consumer child
through the launcher. Both allocations use the same guest address, size, slot
and generation, so the owner field alone distinguishes them. The image also
injects rejection after backing allocation and simulated fixture setup failure
in both the guest child and the freestanding interpreter. These setup probes
run after guest entry, when the shim can own pages. Existing kernel loader and
graph rollback controls exercise failures before guest entry.

```sh
export CARGO_NET_OFFLINE=true GIT_ALLOW_PROTOCOL=file
bash tools/helios-dynamic-elf/build.sh malloc final
python3 tools/helios-dynamic-elf/replay.py tcg --malloc final
python3 tools/helios-dynamic-elf/replay.py tcg --malloc final --replay
python3 tools/helios-dynamic-elf/replay.py kvm --malloc final
```

Evidence is under `target/helios-runtime-malloc/final/`. The gate checks the
locally built artifact manifest and ISO hashes, uses two vCPUs and disables the
guest NIC. Each boot has its own `result.json`, guest operation log and host
suite logs. The replay verifies the ISO SHA-256 against the initial run.
Same numeric PID recycling remains **not exercisable under the current
allocator contract**. Wayland and transport were not rerun. Qt and Qalculate!
remain unlaunched.

### Verified matrix

| Boot | Guest shim and regression controls | Kernel host | ELF host | PID allocator | Page allocator | Malloc shim |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Initial two-vCPU TCG | PASS | 212/212 | 14/14 | 2/2 | 7/7 | 8/8 |
| Same-image two-vCPU TCG replay | PASS | 212/212 | 14/14 | 2/2 | 7/7 | 8/8 |
| KVM | NOT RUN — /dev/kvm unavailable | Unrun | Unrun | Unrun | Unrun | Unrun |

Both boots verified malloc/free, zeroing, invalid and foreign handles,
exhaustion, mapping and metadata rollback, recoverable unmap failure, and
live-allocation teardown. Regression controls covered mmap, the page allocator,
ELF, application and shared-object TLS, direct TLS resolver rejection,
lifecycle, waitpid and graph rollback.

- ISO SHA-256 `9d46d035e162c73ac3d672918fea89bfc99b9e5c8bbfdcba37f2219728a1d528`
- Stamped malloc and mmap fixture SHA-256 `416452d2232ab11c9936340e0a5315f6102457b48671519bf25c114d0e8c7e68`
