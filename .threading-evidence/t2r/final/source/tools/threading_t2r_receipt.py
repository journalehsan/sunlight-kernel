#!/usr/bin/env python3
"""Strict pilot receipts only; passing a pilot does not close a T2 race row."""
import re


def assess_pilot(text, case, cpus):
    if not text.endswith("\n"):
        return False
    def rows(pattern):
        return re.findall('^' + pattern + '$', text, re.M)
    if any(bad in text for bad in ('KERNEL PANIC', '[EXCEPTION]', 'T2R pilot no', 'T2R ring3 failed')):
        return False
    if rows(r'\[T2R\] begin case=(\d+) cpus=(\d+) bounded_entry_spins=2000000') != [(str(case), str(cpus))]:
        return False
    actors = rows(r'\[T2R\] registered actor=(\d+) pid=(\d+) incarnation=(\d+) vm=AddressSpaceIdentity \{ pml4_phys: (\d+), generation: (\d+) \}')
    expected = {0} if case == 2 else {0, 1}
    if len(actors) != len(expected) or {int(a[0]) for a in actors} != expected:
        return False
    if len({a[1:3] for a in actors}) != len(expected):
        return False
    if (case == 1 and len({a[3:] for a in actors}) != 1) or (case == 0 and len({a[3:] for a in actors}) != 2):
        return False
    entries = rows(r'\[T2R\] entry actor=(\d+) cpu=(\d+) mask=3 outside_lifecycle_lock=1')
    if case != 2 and (len(entries) != 2 or {int(a) for a, _ in entries} != expected or len({c for _, c in entries}) != 2 or any(int(c) >= cpus for _, c in entries)):
        return False
    commits = rows(r'\[T2R\] commit actor=(\d+) parent=(\d+) tid=(\d+) incarnation=(\d+) slot=(\d+) vm=AddressSpaceIdentity \{ pml4_phys: (\d+), generation: (\d+) \} stack=true queued=1 workers=(\d+)')
    rejects = rows(r'\[T2R\] reject actor=(\d+) pid=(\d+) incarnation=(\d+) error=Capacity workers=(\d+) reservation=none')
    if len(commits) != 1 or len(rejects) != int(case != 2):
        return False
    c = commits[0]
    requested = rows(r'\[T2R\] requested-winner actor=([01])')
    if len(requested) != 1 or (case != 2 and requested[0] != c[0]):
        return False
    a = next((a for a in actors if a[0] == c[0]), None)
    if a is None or a[1] != c[1] or a[3:] != c[5:7] or int(c[7]) != (96, 32, 1)[case]:
        return False
    if rejects:
        r = rejects[0]
        a = next((a for a in actors if a[0] == r[0]), None)
        if a is None or a[1:3] != r[1:3] or r[0] == c[0] or r[3] != c[7]:
            return False
    retired = rows(r'\[T2R\] retired tid=(\d+) incarnation=(\d+) slot=(\d+) state=Reaped stack=none pinned=false progress=([1-9]\d*)')
    if len(retired) != 1 or retired[0][:3] != c[2:5]:
        return False
    if rows(r'\[T2R\] outcome case=(\d+) successes=1 commits=1 rejects=(\d+) entry_mask=(\d+)') != [(str(case), str(int(case != 2)), '0' if case == 2 else '3')]:
        return False
    resources = rows(r'\[T2R\] resources point=drained workers=0 occupied=(\d+) slots=(\d+) stacks=(\d+) fd_refs=(\d+) cap_entries=(\d+) heap=(\d+) reserved=0 pending=0')
    # Four registered owners stay live until the final machine receipt.
    if len(resources) != 1 or resources[0][0] != '4' or resources[0][2:5] != ('4', '12', '4'):
        return False
    if case == 2:
        exits = rows(r'\[T2R\] child-exit tid=(\d+) incarnation=(\d+) cpu=(\d+) stack_live=true')
        if len(exits) != 1 or exits[0][:2] != c[2:4] or int(exits[0][2]) >= cpus:
            return False
    baseline = rows(r'\[T2R\] baseline heap=(\d+) workers=0 stacks=4 fd_refs=12 cap_entries=4 reserved=0 pending=0')
    heap = rows(r'\[T2R\] heap baseline=(\d+) cached_fd_boxes=(\d+) cached_fd_bytes=(\d+) reaped_stacks=(\d+) drift=0')
    if len(baseline) != 1 or len(heap) != 1 or baseline[0] != heap[0][0]:
        return False
    base, boxes, cache_bytes, reaped = map(int, heap[0])
    if boxes != (96, 32, 1)[case] or boxes != reaped or cache_bytes != boxes * 10240 or int(resources[0][5]) != base + cache_bytes:
        return False
    vms = rows(r'\[T2R\] vm owner=(\d+) incarnation=(\d+) identity=AddressSpaceIdentity \{ pml4_phys: (\d+), generation: (\d+) \} borrowers=0')
    if len(vms) != 4 or len(set(vms)) != 4:
        return False
    return rows(r'\[T2R\] PILOT PASS case=(\d+) cpus=(\d+)') == [(str(case), str(cpus))]
