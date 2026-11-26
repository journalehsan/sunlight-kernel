# TTY and PTY functional qualification

The original “delayed TTY producer startup” failure was a fixture
readiness error. On fresh 512 MiB BIOS guests, the original fixture
passed the 65,536-byte integrity test and then exited 1 with both eager
and adaptive TTY heaps. The producer had executed and created its
marker. The fixture polled routed path metadata, which returned
`ENOENT` for a mutable `/tmp` file created through the kernel file path
under the compressed boot configuration. The marker was readable
through the ordinary file interface. This diagnosis does not establish
a general VFS namespace coherence guarantee.

The native fixture now opens and reads the marker directly, requires a
complete PID record, and keeps the original two-second deadline. The
next run exposed another fixture defect: a successful explicit wait
consumed the child's exit result, and the destructor tried to kill and
wait for the same child again. The fixture now records that it has
collected the child and closes the PTY session separately when needed.
Both changes are confined to the native test fixture. The TTY build
defaults to the adaptive allocator; disabling default features selects
the eager control. The original 16 MiB budget and docserv's adaptive
heap remain in place.

Fresh two-vCPU QEMU 11.1.3 TCG guests booted from optical media with
the opt-in compressed resource module and no NIC. The repaired gate
passed at 512 MiB BIOS, 512 MiB UEFI, 768 MiB BIOS, and 1 GiB BIOS.
The test required
boot and document markers, a successful native suite exit and all
expected serial markers, two independently responsive tabs, and a
working command on the remaining tab after Ctrl+W. The native suite
checked the original 64 KiB transfer, delayed drain, nonblocking
partial writes and retry, concurrent 256 KiB PTY streams, foreground
input and output, interrupt and closure behavior, eight ordinary
shell-session exits, and two byte-checked sweeps from 1 KiB through
1 MiB. The input coverage is interactive input and repeated commands;
it does not characterize sustained bulk-input throughput.

The framebuffer check sends known output before and after an
`ls /bin` listing. OCR on the Home capture must show the standalone
line `ancient meadow` and omit `fresh garden`; the End capture must
show the opposite. This verifies visible history and live-view
selection for the bounded fixture, rather than comparing every glyph
or every retained line.

The adaptive heap mapped 6,553,600 bytes at login, 7,077,888 with
one tab and 7,602,176 with two. Closing the second tab returned 524,288
bytes of mapped capacity. In 512 MiB BIOS, the kernel reported reaping
that tab's shell with 905 user frames and 10 page-table frames. The
native suite separately checked session process and descriptor cleanup.
The 512 MiB BIOS command response samples were 32–64 ms for ordinary
commands; the full nested suite took 15,170 ms. These times measure
Enter submission to serial completion and include host scheduling and
polling. They do not measure key-to-pixel latency.

The [results JSON](tty-pty-functional-qualification-results.json)
contains the per-guest verdicts, command timings, stream timings, heap
samples and evidence paths. The bounded runs do not establish
sustained 512 MiB operation, general scrollback correctness, or
arbitrary memory-pressure recovery.
