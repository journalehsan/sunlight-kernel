# SunlightOS compared with other microkernel systems

Snapshot: 2026-11-01, SunlightOS commit `52d8fabef`. This is a source-code review, not a benchmark or certification. SunlightOS observations describe code present in this checkout; they do not establish that every path works in every configuration. No boot or compatibility tests were run for this document. External descriptions refer to the linked project documentation, not a particular measured build.

The comparison includes **seL4**, **GNU Mach with GNU Hurd**, and **Zircon with Fuchsia**. The latter two include operating-system services above the kernel: application compatibility and drivers cannot be compared fairly at the kernel level alone. Zircon describes itself as a kernel plus a small userspace foundation, so its precise classification varies by definition.

## At a glance

| Area | SunlightOS | seL4 | GNU Mach / Hurd | Zircon / Fuchsia |
| --- | --- | --- | --- | --- |
| Design | Microkernel-oriented desktop; kernel includes scheduler, IPC, capabilities and substantial memory management. | Minimal capability-based microkernel; much resource policy is in userspace. | Mach microkernel with Hurd userspace servers. | Zircon kernel and userspace foundation beneath Fuchsia. |
| Physical and virtual memory | Kernel frame allocator and page-table manager; per-process address spaces, shared memory and compressed swap code. | Kernel provides mapping primitives; userspace manages most physical memory and mapping policy. | Kernel VM objects and mappings, with external memory-manager/pager interfaces. | Process address spaces use VMARs and virtual memory objects (VMOs). |
| IPC | Capability-authorized endpoints, bounded queues, messages and reply handling; shared-memory capabilities for larger data. | Synchronous endpoint IPC, capabilities and notifications; userspace can set up shared memory. | Port rights and messages; Hurd servers use RPC built on Mach IPC. | Asynchronous channels carry messages and handles; synchronous calls can be built on channels. |
| Access control | Kernel capability broker provides rights, derivation, revocation and ownership checks for several resource types. | Kernel-protected capabilities control access to kernel objects; proofs cover selected configurations and properties. | Port rights provide authority to communicate with objects and servers. | Processes use handles with rights to access kernel objects. |
| Application compatibility | Native ABI plus a **partial** x86-64 Linux syscall translation layer; behavior depends on the syscall and program. | No Linux ABI implied by the microkernel; Linux can run in a VM on supported configurations. | Hurd servers and GNU libraries provide Unix/POSIX interfaces; this is not Linux binary ABI compatibility. | Fuchsia interfaces and libraries sit above Zircon; Linux compatibility depends on additional system components, not Zircon alone. |
| Scheduling and CPUs | Per-core scheduler code, affinity and work stealing; current source selects round-robin mode. SMP implementation exists; scalability is not established here. | Thread scheduling; multicore support depends on platform and configuration. | Kernel tasks and threads; Hurd provides higher-level process semantics. | Kernel manages processes and threads; Fuchsia provides higher-level services. |
| Drivers | Userspace driver and service code exists, alongside kernel hardware code. The separate `drivers` framework crate is a stub. | Drivers can be placed in userspace; hardware and DMA isolation depend on platform and configuration. | Hurd services run in userspace, but GNU Mach also contains drivers, including Linux-derived drivers. | Fuchsia uses userspace services and drivers alongside Zircon. |
| Assurance | Tests and diagnostics exist in the tree; no formal verification claim is made here. | Formal proofs exist for specific configurations, properties and assumptions. | No comparable formal verification claim assessed here. | No comparable formal verification claim assessed here. |

Sources: [seL4 resource management and FAQ](https://docs.sel4.systems/projects/sel4/frequently-asked-questions), [seL4 IPC](https://docs.sel4.systems/Tutorials/ipc.html), [GNU Mach overview](https://www.gnu.org/software/gnumach/), [Mach external pager](https://www.gnu.org/software/hurd/microkernel/mach/external_pager_mechanism.html), [Zircon overview](https://fuchsia.dev/fuchsia-src/concepts/kernel), [Fuchsia channels](https://fuchsia.dev/fuchsia-src/get-started/learn/fidl), [Zircon processes](https://fuchsia.dev/fuchsia-src/concepts/process/overview), and [seL4 supported and verified platforms](https://docs.sel4.systems/Hardware/).

## How SunlightOS works

```text
Application / Linux-compatible process
          | native syscall or Linux syscall translation
          v
Kernel: scheduler + process/address-space management
        capability broker + IPC endpoints + memory management
          | capability-checked messages / shared-memory mappings
          v
Userspace services and driver code (for example, filesystem and networking)
```

This is a simplified view. Some hardware support also lives in the kernel, and not every syscall or service request follows the same path.

| Component | Evidence in this checkout | What it means for the comparison |
| --- | --- | --- |
| Physical and virtual memory | [Frame allocation](../kernel/src/memory/pmm.rs), [page mapping](../kernel/src/memory/vmm.rs), [process address spaces](../kernel/src/process/address_space.rs) | SunlightOS contains memory-management mechanisms in the kernel; it does not adopt seL4's userspace-owned physical-memory model. |
| Memory pressure | [Anonymous-page reclaim and swap](../kernel/src/memory/swap.rs), [ZRAM storage](../kernel/src/memory/zram.rs) | Compressed in-memory swap and reclaim are implemented in source. Activation depends on policy; this does not demonstrate a disk-backed swap facility. |
| Shared memory | [Shared region allocation and mapping](../kernel/src/memory/shared.rs), [capability broker](../kernel/src/capability/mod.rs) | Shared regions provide a route for bulk data without putting all payload bytes into IPC messages. Lifetime and access controls still matter. |
| IPC | [Endpoint and reply implementation](../kernel/src/ipc/mod.rs), [message layout](../kernel/src/ipc/message.rs) | The internal message has up to eight words and two capability tokens; the compact register ABI carries up to four words. Endpoint and deferred-reply capacities are each 64. These are code limits, not throughput measurements. |
| Capabilities | [Broker, rights, mint, derive and revoke](../kernel/src/capability/mod.rs) | Resource access has explicit capability machinery. The existence of these mechanisms alone does not prove complete mediation or security. |
| Scheduler | [Scheduler implementation and mode](../kernel/src/sched/mod.rs), [SMP bring-up](../kernel/src/arch/x86_64/smp.rs) | Round-robin is selected in the checked-in constant; a burst-oriented mode and per-core work-stealing code are also present. Source comments describe intended behavior, not a measured fairness or latency guarantee. |
| Linux compatibility | [Syscall translation table](../compat-linux/src/lib.rs), [kernel dispatch](../kernel/src/arch/x86_64/syscall.rs), [ELF loader](../kernel/src/process/elf_loader.rs) | Some Linux x86-64 syscalls map to native calls or special handlers; unknown calls return ENOSYS. A mapping does not by itself establish full Linux semantics or application compatibility. The compatibility crate also labels its POSIX open-access shim as a mock. |
| Drivers and services | [Driver framework stub](../drivers/src/lib.rs), [virtio code](../sunlight-virtio/src/lib.rs), [kernel hardware code](../kernel/src/arch/x86_64/), [services](../services/) | Do not infer a complete isolated driver framework from the architecture goal. Individual drivers and their privilege boundaries need separate review and testing. |

## What the differences imply

1. **Memory policy and trusted code:** SunlightOS currently keeps allocation, mapping and compressed-swap machinery in the kernel. seL4 delegates more resource management to userspace, which changes both the trusted code boundary and the work required of system services. Neither design alone establishes lower memory consumption or better performance. See [seL4 untyped memory](https://docs.sel4.systems/Tutorials/untyped.html) and [mapping](https://docs.sel4.systems/Tutorials/mapping.html).
2. **IPC trade-offs:** SunlightOS combines bounded endpoint queues, inline messages, capability transfers and shared memory. seL4 emphasizes synchronous endpoint operations; Mach uses port-based messaging; Zircon offers channels with handle transfer. These interfaces have different semantics, so message size or queue depth alone cannot rank latency. See [seL4 IPC](https://docs.sel4.systems/Tutorials/ipc.html), [Mach ports](https://www.gnu.org/s/hurd/microkernel/mach/port.html), and [Zircon channels](https://fuchsia.dev/fuchsia-src/reference/kernel_objects/channel).
3. **Compatibility is a system property:** SunlightOS's Helios path targets Linux x86-64 syscalls but supports a subset. GNU Hurd's POSIX interface comes from servers and libraries; seL4 can host Linux in a VM on supported hardware. Source-level portability, POSIX conformance and running an unchanged Linux binary are distinct goals. See [GNU Mach introduction](https://www.gnu.org/software/hurd/gnumach-doc/Introduction.html) and the [seL4 FAQ](https://docs.sel4.systems/projects/sel4/frequently-asked-questions).
4. **Assurance is configuration-specific:** seL4's verification is a significant differentiator, but proofs have explicit scope and assumptions and do not automatically cover every configuration or userspace service. SunlightOS capability and test code should not be presented as equivalent formal assurance. See [seL4 platform verification status](https://docs.sel4.systems/Hardware/).

## What to measure next

| Question | Comparable measurement | Evidence needed |
| --- | --- | --- |
| Memory footprint | Kernel image size, boot-time resident memory, per-process and per-endpoint overhead, and peak under a fixed workload. | Same architecture, RAM, build mode and enabled services; distinguish reserved from actually used memory. |
| IPC cost | One-way and round-trip latency distributions, throughput, and CPU use for inline messages and shared-memory payloads. | Message sizes, core placement, queue depth, timer method, sample count and p50/p95/p99; compare equivalent semantics. |
| Linux compatibility | Pass/fail results for named binaries and syscall tests, including unsupported and partially implemented behavior. | Exact binary, dependencies, architecture, test inputs and output; report coverage separately from POSIX conformance. |
| Reliability and isolation | Service failure/restart tests, capability revocation, unauthorized access attempts and memory-pressure recovery. | Reproducible cases, expected outcomes, test configuration and logs; include DMA isolation when hardware is involved. |
| SMP behavior | Scaling, fairness and tail latency across core counts under CPU-bound and IPC-heavy loads. | CPU topology, affinity, workload, configuration, and single-core baseline. |

The repository's [QEMU test script](../tools/test.sh) contains serial-output gates and named test modes; it can validate selected boot paths but is not a substitute for the cross-system measurements above. Run the relevant mode and record the commit, configuration and result before claiming a feature is verified.
