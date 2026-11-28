# VFS-C2I2: Scalable executable sources

**PARTIAL.** The authenticated C2G channel now delivers executable inputs
above 1 MiB to the existing ELF parser and segment planner in the copied
host integration. Ordinary boot remains **BLOCKED** at the unchanged
ownership guard. Native and Helios guest execution are **NOT_RUN**.

## Source and budget

Embedded init, the VFS and TTY bootstrap images, base services launched
through the init spawn capability, and the built-in sunlightd service
programs still use immutable embedded inputs. Their provenance and
associated privileged grants are unchanged. The service snapshot is for
mutable executables in the existing authenticated Downloads route,
including relative paths from a retained Downloads cwd. That route
fails closed; acquisition does not fall back to the kernel filesystem.
The future mutable runtime root would need its own authorized routing
through the same authority before these startup programs could use it.

The service captures a regular file under the same namespace lock as
lookup, authorization, writes, truncation and rename. It retains the
exact backend handle and the captured stat. It returns a bound token
and exact length and provides 4 KiB positional reads of the captured
image. The filename or backing file can change after OPEN without
changing the returned contents or captured metadata. Existing traversal,
read and execute checks, receipt, registration epoch, mount confinement
and ELF/W^X checks remain in place.

The per-source limit is **2,097,152 bytes**. One snapshot slot per
service namespace caps aggregate mapped payload at 2 MiB. Native VFS
uses the existing eager anonymous mmap path, with checked page rounding
and a non-merging region for exact munmap; it does not allocate the
payload from the service's 256 KiB fixed heap. The kernel reserves only
the reported length from its existing heap with a fallible reservation
and holds one buffer slot through eager loading. Maximum simultaneous
payload backing is **4 MiB**, excluding allocator, mapping and channel
metadata and destination ELF pages. The single parked acquisition
constraint is unchanged. The size limit covers the measured TLS and
dialogd images. Larger optional applications remain outside this budget.

## Startup inventory

Whole ELF artifact lengths in bytes. The init list and default managed
units are from the checked-out sources. The local freestanding release
directory supplied the sizes except for dialogd and braind, which use
the custom Sunlight target. Candidate VFS lengths vary with boot
resource configuration.

| Source | Artifacts and lengths in bytes |
| --- | --- |
| Direct bootstrap | sunlight-init 45,568; sunlight-tty-server 3,314,704; default sunlight-vfs-server 88,855,088 |
| Candidate VFS bootstrap | Uncompressed 89,080,368; compressed resource configuration 504,368 |
| Init base list | sunlight-timer-server 12,712; sunlight-kbd 12,712; sunlight-mouse 20,904; sunlight-usb-mouse 21,040; sunlight-swapd 12,792; deviced 500,496; sunlightd 147,968; networkd 45,616; resolved 25,088; powerd 20,984; thermald 29,184; audiod 696,832; sunlight-display 999,984; mezzo 25,088; pty_server 33,280; net_server 227,944 |
| Init fstab command | storagectl 37,376 |
| Default sunlightd units | timezone_service 94,768; timed 45,584; niced 49,672; gcd 45,568; uac_service 127,536; sunlight-sessiond 127,496; sunlight-sm 41,472; sunlight-kv 74,392; sunlight-clipd 49,712; wiseowl-memoryd 123,440; wiseowl-memorydb 410,160; wiseowl-indexd 291,376; wiseowl-braind 223,984; sunlight-dialogd **1,889,096**; rand_service 16,888; sunlight-account-broker 107,144; sunlight-tls **1,217,328** |

None of these embedded startup inputs currently require a service
snapshot. The TLS regression publishes the real bytes as an authorized
mutable test file in Downloads; it does not change how TLS is launched
or grant the test copy TLS service identity.

## Settlement and evidence

The existing OPEN publication reserves a compensating CLOSE. A capture
failure drops the mapping and its slot and closes the retained handle.
If kernel allocation or a PREAD fails after OPEN, the reserved CLOSE
releases the service description. On success the source settles before
image construction; the kernel buffer is freed as soon as the eager
loader returns, before final publication. Earlier errors, including bad
ELF and address-space setup failure, drop the buffer on unwind from the
syscall. The existing canceled-publication, caller retirement and
registration withdrawal paths settle or invalidate the exact source;
the withdrawn waiter drops its buffer when it resumes or is retired.

The real copied service dispatcher and kernel publication runtime passed
exact-byte comparisons and existing loader planning for a small native
ELF, the old boundary, a 1,217,328-byte native and Linux fixture, the real
TLS artifact, and the new 2 MiB boundary. The test also exercised
oversize rejection, injected kernel reserve failure after service
capture, failed/short PREAD, rename and replacement, mutation and
truncation between reads, discarded OPEN, cancellation, withdrawal,
bad ELF and repeated settlement. The test checks the handle inventory,
file publication inventory and absence of a legacy source handle.

For TLS the host exercised 1,217,328 bytes per copy, or 2,434,656
simultaneous logical payload bytes. The native mapping would round
to 1,220,608 bytes (298 pages) by arithmetic. Live guest service heap
baseline, native mapping peak and reclamation, PMM behavior under low
memory, native mmap allocation fault injection and actual address-space
construction, rollback and dispatch are **NOT_RUN**. The host uses a heap
backing for the service snapshot and does not measure guest heap or PMM.

## Ownership

Inventory **PASS**, 71 references before and after, with identical
normalized inventories. No-growth and final removal both **FAIL**
before and after. The four unmatched historical findings remain:

| Finding | Remaining use |
| --- | --- |
| `storage::mount` | Legacy FAT constructor for unreserved media and unclaimed targets. |
| `linux_procfs::mountinfo_snapshot` | Legacy mount topology. |
| `sys_linux_statfs` | Legacy capacity for unrouted Linux callers. |
| `process::mmap::sys_mmap` | Private file mappings from legacy descriptions. |

Whole runtime root authority, generic native/Linux filesystem routing,
and migration of the remaining legacy producers still gate activation.
See [machine-readable results](vfs-scalable-executable-sources-results.json).
