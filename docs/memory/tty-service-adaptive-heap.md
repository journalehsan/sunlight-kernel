# Adaptive TTY service heap

TTY now maps **6.25 MiB at login, down from 16 MiB**, and retains its
16 MiB allocation budget. Before any login, its one live allocation
holds the decoded 1448 × 1086 wallpaper for redraw and re-login:
6,290,112 requested bytes and 6,290,168 bytes with allocator overhead.
Those figures were unchanged at idle and after switching allocators.

The native TTY feature reuses docserv's extent allocator with its own
budget and first extent size. The initial extent leaves about 257 KiB
above observed login backing. Subsequent mappings target 256 KiB, or
the page-rounded request with alignment overhead for larger allocations.
The allocator tries existing free lists first, retries a failed growth
mapping at the minimum required size when smaller, and unmaps wholly
free growth extents. It retains the initial extent. The aggregate
16 MiB ceiling includes all mapped extents. Segmentation can leave free
space in different extents, so this ceiling does not guarantee the
same maximum contiguous allocation as the old arena.

No tab, scrollback, PTY protocol, wallpaper, or input routing behavior
was changed. Docserv retains its own 256 KiB initial/growth target and
128 MiB cap. Native anonymous mappings, the 16 MiB kernel heap, PMM
accounting, ZRAM, TLS, stacks, and VFS prerequisites were left intact.
The opt-in compressed module hash matched the previously audited module
with 613 entries and 154,184,731 logical bytes.

## Measured TTY demand

Bytes sampled by the opt-in heap probe. Peak is the allocator's running
high water for backing bytes, which includes headers and alignment.

| Checkpoint | Eager mapped | Adaptive mapped | Live requested | Live backing |
| --- | ---: | ---: | ---: | ---: |
| Login ready and idle | 16,777,216 | 6,553,600 | 6,290,112 | 6,290,168 |
| One shell tab | 16,777,216 | 7,077,888 | 6,816,720 | 6,817,000 |
| Two tabs and output | 16,777,216 | 7,602,176 | 7,343,328 | 7,343,832 |
| Second tab closed | 16,777,216 | 7,077,888 | 6,816,720 | 6,817,000 |

Peak backing in the bounded terminal workload was 7,345,744 bytes.
Closing the second tab released 512 KiB of mapped capacity, and the
remaining tab answered another command. All four guests also passed a
fallible allocator probe: grow a 1 MiB buffer, reject an over-budget
reserve, verify the original bytes, then drop the buffer and verify
capacity and live backing returned to their pre-probe values. This
tests budget rejection and growth cleanup; it does not inject a kernel
mmap failure or make infallible allocations recoverable.

## Native validation

Fresh optical-only QEMU 11.1.3 TCG guests used two vCPUs and no NIC.
All four passed ordinary boot and service gates, decoded the login
wallpaper, logged in to native TTY, ran and switched between two shell
tabs, drained foreground output, and closed a tab. The workload sent
Page Up, Home and End; it did not assert the rendered scrollback contents.
The existing document workload passed in each guest before login.

Physical counts refer to 4 KiB frames. The first column is the steady
pre-document-workload checkpoint after the boot idle interval. The
minimum covers boot through document recovery, before the tab workload.

| Guest | Free at doc idle | Minimum through doc recovery | Free at doc recovery |
| --- | ---: | ---: | ---: |
| Eager 512 MiB BIOS | 14,269 | 1,455 | 14,269 |
| Adaptive 512 MiB BIOS | 16,769 | 3,955 | 16,769 |
| Adaptive 512 MiB UEFI | 15,725 | 2,911 | 15,725 |
| Adaptive 768 MiB BIOS | 82,305 | 69,491 | 82,305 |
| Adaptive 1 GiB BIOS | 147,841 | 135,027 | 147,841 |

The matched BIOS runs show 2,500 more free frames at idle, exactly
2,496 fewer user-private frames and four fewer page-table frames.
Managed frames equal free plus classified frames at the sampled
checkpoints. This is a guest-wide physical comparison; the heap probe
reports TTY mappings and allocations, not per-command guest RSS.

The extended 768 MiB run completed twelve successive `ls /bin`
commands and passed the shell suite's 65,536-byte PTY integrity check.
The suite then exited 1 at “delayed TTY producer startup,” while the
runner was waiting for its `/tmp` marker. PTY owner-death cleanup
appeared in the log. The cause was not isolated, and the full PTY
liveness, concurrent-stream, and delayed-drain gates remain open.
The sampled TTY heap remained at 7,602,176 bytes mapped with two tabs
during the extended run.

The first matrix measures wall time from submitting Enter until the
serial result is observed, including a 150 ms input pacing floor. The
follow-up timing driver polls every 10 ms and separates that pacing
from the command result. Across eight commands in the matched 512 MiB
BIOS follow-up runs, the eager heap took 32–77 ms and the adaptive
heap took 33–55 ms. These are command response measurements;
they do not measure key-to-pixel latency. See the [results JSON](
tty-service-adaptive-heap-results.json) for individual observations.

The ordinary service release builds and ordinary kernel release check
passed. Nine host static allocator tests passed when run serially;
parallel allocator tests interfered through their shared heap. The GNU
host run of the dynamic profile could not exercise native mmap and
failed its allocation assertions. The adaptive allocation evidence
comes from the native guests.

For reproduction, build TTY with `--features heap-probe` and the
userspace linker flags, build docserv with
`DOCSERV_FEATURES=native-heap-probe bash tools/build_docserv.sh`,
then build the opt-in image with
`bash tools/build_boot_resource_prototype.sh`. Run
`python3 tools/qualify_tty_heap.py ISO OUT --ram 512`; repeat with
`--firmware` pointing at OVMF for UEFI and with 768 and 1024 for BIOS.
The extended suite additionally requires a native shell built with
`native-tests`. The bounded results do not qualify sustained
512 MiB operation, full scrollback correctness, or arbitrary OOM
recovery.
