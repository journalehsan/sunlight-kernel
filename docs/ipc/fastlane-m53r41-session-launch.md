# M5.3R4.1 Account-authorized session launch

Implementation and the listed native launch/inheritance matrix are qualified.
Production FastLane, restricted-channel admission and hostile-process zero-copy
remain disabled. This does not qualify the older 28 scheduled mapping/lifecycle
cases or general shared-service charge delegation.

Evidence: `.fastlane-evidence/m53r4.1-20261109T183520Z/REPORT.md` and
`validation-summary.json`. Earlier M5.3R4 evidence remains unchanged.

The existing local Auth proof and Account registry remain the authority. A fresh
registration starts suspended with one-use bootstrap eligibility. Only the exact
trusted SessionD owner (PID, incarnation, VM generation) can obtain the opaque
initial launch attachment. Commit rechecks the session reference, deadline,
owner and launch revision, consumes bootstrap eligibility and activates the row.
An explicit suspension closes bootstrap and invalidates previously issued launch
permits, including after reactivation. Numeric session references alone confer
no launch authority.

SessionD shell and optional-app launches now use this path. Construction stays
private under the existing scheduler/PMM/broker transaction; membership precedes
publication and runnable admission. Task-table and queue capacity are reserved
before construction. Publication and enqueue form the non-failing commit tail.
Four debug construction checkpoints and a rejected invalid-ELF child demonstrate
VM-frame restoration without publishing a failed child. The latter also closes
an initial-root leak when ELF rejection precedes replacement-VM construction.

Native process spawn, IPC child spawn, native threads and supported Linux thread
clone derive membership from current parent kernel state. Children get fresh
incarnations/VM identities and retain the parent's current credentials and
permitted lookup profile. IPC children now have the correct parent PID. Login
credentials from the registry must not restore privileges dropped by a parent.
Threads share their process quota owner and Account session; Linux clone now
sets that owner explicitly. Unsupported Linux process fork/clone remains denied.

Exec preserves membership and process incarnation while replacing the VM. The
existing MM-0 contract rejects exec while borrowers remain live; the fixture
waits for kernel thread quiescence before retrying. Owner exit terminates live
borrowers through the existing scheduler lifecycle.

SHM creation requires live Account membership or an explicit kernel-admitted
system allocation policy. UID zero alone does not establish that policy. Boot
services and their authorized system descendants retain their service policy;
authenticated applications lose system policy and service provenance, including
root applications. Legacy authenticated spawning without an Account attachment
is denied; CLI callers of that legacy path need a SessionD handoff rather than
a sessionless fallback. General shared-service requester charging is unsupported.
The unchanged 512 MiB/process and 1 GiB/Account-session limits cover SHM backing,
not all anonymous memory, file mappings or page tables.

Final-source QEMU runs on 1/2/4/12 CPUs pass initial launch, first userspace
membership, native and IPC inheritance, privilege-drop preservation, Linux
clone, thread exit, owner exit with a live thread, exec preservation, suspension
fencing, stale-generation denial, bootstrap replay denial, foreign-authority
denial, retirement fencing and construction rollback. A separate source-matched
image passes the native quota prerequisites on all four CPU counts. TCG results
are functional qualification, not performance measurements.
