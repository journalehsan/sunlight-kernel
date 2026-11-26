# System services memory footprint

The largest confirmed eager service arenas are TTY (16 MiB) and TLS (8 MiB). The largest common cost is the fully backed 2 MiB main stack: 66 MiB across the 33 BIOS service leaders at the idle checkpoint. Start with TTY arena instrumentation and adaptive allocation; stack backing has more potential savings but changes a wider kernel contract.

Four fresh optical-only QEMU 11.1.3 TCG guests used two vCPUs, no NIC or disk, and the opt-in compressed boot module. The audit ISO adds only serial census instrumentation and uses the existing document heap probe. The module independently matched all 613 entries and 154,184,731 logical bytes against the audited seed. The original probe ISO was also rerun as a control. No production service, package, allocator, 16 MiB kernel heap, PMM equation, ZRAM policy, or VFS prerequisite changed.

## Native headroom

Free 4 KiB frames; the minimum covers startup and the bounded document workload. Session readiness, bundle discovery and document readiness can interleave.

| Guest | PMM entry | Pre-scheduler | Phase 3 | Namespace | Session ready | Bundles | Doc ready | 10 s idle | Recovered | Minimum |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| bios-1024 | 178763 | 172092 | 167983 | 167980 | 151553 | 151023 | 152635 | 145345 | 145345 | 132531 |
| bios-512 | 47691 | 41020 | 36911 | 36908 | 19951 | 19409 | 19951 | 14273 | 14273 | 1459 |
| bios-768 | 113227 | 106556 | 102447 | 102444 | 85487 | 84323 | 86038 | 79809 | 79809 | 66995 |
| uefi-512 | 46107 | 39444 | 35335 | 35332 | 19437 | 18907 | 19988 | 13229 | 13229 | 415 |

All four passed namespace, shared VFS read, swap policy, session readiness, bundle discovery and document RPC gates. The control ISO had two more free frames at idle in each guest. All sampled PMM class totals plus free frames equal managed frames. The instrumented UEFI guest reclaimed 12 pages by recovery, with 12 compressed backing frames and no successful swap-in at that checkpoint. The 512 MiB guests reached about 5.7 MiB (BIOS) and 1.6 MiB (UEFI) free at the workload minimum. These runs do not qualify sustained 512 MiB operation, a logged-in desktop, or concurrent application workloads.

## Service inventory and ownership

The table shows the 512 MiB BIOS idle census. The [results JSON](system-services-memory-footprint-results.json) includes all four guests and the startup, idle and recovery snapshots. Every row has a 512-page main user stack ([layout](../../kernel/src/process/layout.rs), [backing](../../kernel/src/process/spawn.rs)). Each leader has a 32 KiB kernel stack allocated inside the already backed kernel heap. The document probe also creates a worker with a 512-page user stack in the leader's anonymous mappings and its own kernel stack; the latter is outside the table. The USB mouse process launched under UEFI but exited after reporting no usable xHCI boot mouse; the BIOS row is 521 private pages. The one-shot storagectl and swap policy configurator are startup tasks, not idle residents. The other BIOS idle totals match the 512 MiB BIOS census.

| Service | Present user pages | Private present pages | Other present pages | Anonymous ledger pages | Heap capacity evidence (bytes) |
| --- | ---: | ---: | ---: | ---: | ---: |
| init | 525 | 525 | 0 | 0 | — |
| vfs_server | 21733 | 639 | 21094 | 2 | 262,144 |
| tty_server | 6424 | 5420 | 1004 | 4096 | 16,777,216 |
| timer_server | 514 | 514 | 0 | 0 | — |
| sunlight-kbd | 514 | 514 | 0 | 0 | — |
| sunlight-mouse | 516 | 516 | 0 | 0 | — |
| sunlight-usb-mouse | 525 | 521 | 4 | 0 | — |
| deviced | 634 | 634 | 0 | 0 | — |
| sunlightd | 548 | 548 | 0 | 0 | 65,536 |
| networkd | 523 | 523 | 0 | 0 | — |
| resolved | 518 | 518 | 0 | 0 | — |
| powerd | 517 | 517 | 0 | 0 | — |
| timezone_service | 599 | 599 | 0 | 0 | 262,144 |
| thermald | 519 | 519 | 0 | 0 | — |
| audiod | 682 | 682 | 0 | 0 | — |
| niced | 528 | 524 | 4 | 0 | — |
| sunlight-display | 760 | 760 | 0 | 0 | — |
| mezzo | 518 | 518 | 0 | 0 | — |
| gcd | 527 | 523 | 4 | 0 | — |
| pty_server | 520 | 520 | 0 | 0 | — |
| net_server | 824 | 824 | 0 | 0 | 1,048,576 |
| uac_service | 1055 | 1055 | 0 | 0 | 2,097,152 |
| sunlight-sm | 522 | 522 | 0 | 0 | 65,536 |
| sunlight-dialogd | 975 | 975 | 0 | 2 | — |
| rand_service | 516 | 516 | 0 | 0 | — |
| sunlight-docserv | 2332 | 2332 | 0 | 580 | 262,144 |
| timed | 523 | 523 | 0 | 0 | 262,144 |
| sunlight-sessiond | 543 | 543 | 0 | 0 | — |
| sunlight-kv | 790 | 787 | 3 | 256 | 1,048,576 |
| wiseowl-memorydb | 612 | 612 | 0 | 0 | — |
| sunlight-tls | 2860 | 2860 | 0 | 0 | 8,388,608 |
| sunlight-clipd | 572 | 572 | 0 | 0 | 196,608 |
| sunlight-account-broker | 795 | 795 | 0 | 0 | 1,048,576 |

The page walk counts present user mappings. Private counts require a matching PMM owner PID and UserPrivate class. At idle the service private counts sum to 28,950 BIOS or 28,429 UEFI frames, matching the global class. Other present pages include aliases and device mappings; summing them would overstate shared physical RAM. The region ledger records address ranges, including any reserved, absent or swapped pages. Its categories must not be treated as extra frames on top of the page walk. Heap capacity is measured for docserv, inferred from the matching eager mappings and source for TTY and KV, and taken from source for static arenas. A dash means unmeasured or an uninstantiated configured arena; it does not establish zero allocator overhead.

- **TTY:** The 4,096-page native anonymous arena appears between pre-scheduler and Phase 3, then persists at idle. It is resident, not merely reserved. The 1,000-page framebuffer aperture and four telemetry pages do not belong to its private total. The login background is embedded in the executable; measure scrollback and login asset lifetimes separately before redesigning them.
- **TLS, UAC, account broker and net server:** [TLS](../../sunlight-tls/src/main.rs), [UAC](../../services/sunlight-uac/src/bin/uac_service.rs), [account broker](../../sunlight-account-broker/src/main.rs) and [net server](../../services/net_server/src/main.rs) declare static arenas of 8, 2, 1 and 1 MiB respectively. These are part of eagerly installed ELF pages, not additional anonymous mappings. Arena capacity does not establish live demand. No network workload ran on the optical-only guests.
- **KV, display and dialog:** KV retains its 1 MiB eager mapping. Display declares an 8 MiB dynamic arena but has no anonymous ledger pages at idle; its startup log reports no compositor backbuffer allocation. Dialog declares a 16 MiB dynamic arena; its 8 KiB anonymous range is not sufficient evidence of an allocator arena. Wise Owl memorydb declares a 1 MiB dynamic arena with no observed idle anonymous range.
- **Shared resources:** VFS maps the same 21,092 retained module frames as the kernel. Its boot-shared ledger also includes one additional page. The compressed archive is outside PMM; VFS's 639 private frames include its own stack, ELF and anonymous mappings. Read-only ELF pages are private copies of loadable content from the executable catalog. Sharing them would require compatible alignment, identical bytes, W^X, relocation and lifetime guarantees. The seed already shares its immutable backing. No unused buffer or safe duplicate removal was demonstrated.
- **Allocator metadata:** Docserv reports 714–722 live payload bytes in a 256 KiB arena at readiness. Live backing minus payload includes headers and alignment; it excludes static allocator descriptors and cannot be called pure metadata. The native libc layout has 56-byte allocation headers and 16-byte free-list nodes on x86-64; the adaptive descriptor table has 512 slots at the current 128 MiB capacity. Per-service live allocator metadata, free-list fragmentation and stack high-water use were not measured outside the document allocator probe.

## Request coverage and implementation order

VFS exercised read, stat and write-policy checks at boot, including a 2 KiB shared read. Those checks have phase snapshots, without isolated per-request peaks. Docserv exercised native open/count/render/thumbnail/close calls. Its anonymous footprint grows during rendering and returns to the idle value after the probe; live backing peaks around 33.3 MiB, with the service and probe worker sharing the allocator. Other services have startup and idle residency evidence but lack isolated request measurements. In particular, this audit does not establish that quiet heaps or buffers can be removed.

| Priority | Opportunity | Estimated physical savings | Complexity / risk |
| --- | --- | ---: | --- |
| 1 | TTY: reuse adaptive extents after measuring login, active tabs, scrollback and burst output | Up to 15.75 MiB if 256 KiB is sufficient at idle | Medium / medium |
| 2 | TLS: replace the static arena with a demand-grown arena after handshake, reconnect and concurrent-stream measurements | Up to 7.75 MiB under the same assumption | Medium / medium-high |
| 3 | Keep native stack address ranges and guard pages; investigate committing on access | Illustrative 57.75 MiB if 33 main stacks average 256 KiB committed | High / high |
| 4 | Measure and then stage UAC, account broker, net server and KV arena changes independently | Up to 4 MiB combined under a 256 KiB idle assumption per arena | Medium / medium-high |
| 5 | Share eligible immutable ELF pages with the catalog | Unknown until an aligned-page inventory and loader prototype | High / high |

The current adaptive feature selects the document-specific 128 MiB capacity and descriptor count. Reuse the extent implementation with each service's existing capacity and explicit configuration, rather than enabling that feature unchanged. TTY already uses the native libc allocation ABI and is the closest fit. TLS uses its own LockedHeap and needs an allocator migration or an extent adapter. Preserve allocation-failure behavior, alignment, release of empty growth extents and thread safety. Test fragmentation and simultaneous allocations: an aggregate limit does not guarantee that segmented extents satisfy every allocation pattern the old contiguous arena accepted.

Before changing stack backing, measure real high-water use and cover fault delivery, kernel copies to user buffers, initial argument construction, guard edges, thread exit and reclaim. Before tuning any remaining service, record ready/idle/peak/recovery, distinguish heap payload from allocator backing and metadata, and run representative requests with production devices where needed. Qualifying sustained 512 MiB also requires repeated and concurrent ordinary workloads with a responsive desktop and observed reclaim progress.

## Reproduction

The audit-only patch, exact QEMU arguments, ISO hash, and serial logs are under `target/system-services-memory-footprint/`; use its `census/` subdirectory for this report's measurements. Apply `census.patch` to the recorded revision, rebuild the renderer with `DOCSERV_FEATURES=native-heap-probe bash tools/build_docserv.sh`, then run `bash tools/build_boot_resource_prototype.sh`. Use `python3 tools/qualify_boot_resource_prototype.py` with `--document-heap --ram 512 768 1024` for BIOS, and add `--firmware /usr/share/OVMF/x64/OVMF.4m.fd --ram 512` for UEFI. The audit build samples under the scheduler and PMM locks, so its timing and service readiness order can differ slightly from the control. The temporary patch was removed from the working tree after the audit build.
