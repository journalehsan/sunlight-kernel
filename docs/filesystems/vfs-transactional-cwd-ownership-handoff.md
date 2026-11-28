# VFS-C2F1 transactional cwd ownership handoff

**PARTIAL.** The existing authenticated cwd continuation now preserves the
previous retained binding on rejected publication. The opt-in Downloads
candidate supports distinct-description Linux renameat and nonempty
directory-relative newfstatat in source and through the host copied channel.
The generic legacy/service cwd ownership handoff remains incomplete.

## Cwd publication and compensation

Linux chdir translates to the same native adapter as native chdir.
The selected-profile route parks in the existing authenticated continuation.
The generic legacy route uses kernel lookup and explicitly rejects Downloads
entry. Its legacy cwd commit withdraws caller profiles. The service
continuation requires a live retained namespace and treats its root and cwd
as authoritative. An ordinary legacy cwd string does not supply that
authority. The continuation also lacks an authoritative legacy target
publication step when a traversal leaves Downloads.

The selected-profile continuation revalidates the delivered request, service
registration, caller and waiter identities, credentials and captured process
state. The service keeps a saved reference to the old directory pair while
retaining the new pair. The kernel correlates the retain and constructs the
replacement binding before replacing the exact previous scope. Under the
scheduler guard it updates process cwd and invalidates stale sibling
profiles. Reconciliation queues the old release once local users drain.
The parked wait runs outside the scheduler, broker and VFS guards.

An authenticated rejected finish settles the provisional retain and wakes
the waiter with an error. The service restores its old binding and releases
the provisional pair. The host test verifies that the prior scope can still
serve a lookup and that backend handles return to the two-pin baseline.
Timeout and service retirement still use their existing fail-closed paths.

## Directory descriptions

The Linux renameat adapter checks both description authorities and rights.
The service resolves both tokens independently under the active caller
and registration. Each path is walked from its pinned directory beneath
the retained root under the filesystem lock. Validation precedes mutation,
and a Downloads boundary crossing returns EXDEV. The copied channel test
rejects a closed target token and verifies that the original file remains.

For nonempty relative newfstatat, the kernel copies the pathname and supplies
the selected description token, caller credentials and namespace receipt.
The service validates the receipt and walks from the retained directory.
The kernel validates the completion and copies a Linux stat record. The
copied channel test stats the renamed file relative to the target directory.
Closing both directory descriptions returns the backend handle count to
baseline.

## Guard and qualification

The unchanged source inventory is **PASS** at 71 before and after.
No-growth and final removal are **FAIL**. No-growth compares the multiset of
file, ownership category and exact trimmed source text. It ignores line
numbers. A rewritten reference, an extra reference in a tracked file or
a reference moved to another file can fail comparison even if the total
count stays fixed. The four unmatched entries were already present on
entry and remain identical after this work.

| Historical finding | Why the site is unmatched |
| --- | --- |
| `storage::mount` | The historical FAT mount constructor remains for legacy storage attachment. Its mutable binding differs from the recorded constructor line. |
| `linux_procfs::mountinfo_snapshot` | The historical direct VFS lock for the legacy topology snapshot is absent from the reviewed baseline. |
| `sys_linux_statfs` | Storage telemetry added a direct legacy capacity lookup absent from the baseline. |
| `process::mmap::sys_mmap` | The surviving VFS lock is in a file with no reviewed ownership entries. It remains for legacy private mappings. |

Ordinary boot is **BLOCKED** by the unchanged guard before guest launch.
Native and Linux guest syscall tests and the C2E, C2D, C2B, C1E10–C1E12,
C1E6A and C1E6B guest regression replays are **NOT_RUN**. Host cwd tests
exercise the selected-profile continuation, including retained rename,
failed lookup, stale epoch, retirement and rejected finish. They do not
qualify generic ownership transitions or relative operations across those
transitions. The prepared C2C GPT image passed its existing SHA-256 check.

See the [machine-readable results](vfs-transactional-cwd-ownership-handoff-results.json).
