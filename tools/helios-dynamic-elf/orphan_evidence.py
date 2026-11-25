import re


CONTROLS = ("static", "pie", "relocation", "real", "graph")
OUTPUT = dict(static="[DYNAMIC-ELF] probe write OK",
              pie="[HELIOS-HANDOFF] interpreter entry verified; application entry not called",
              relocation="[HELIOS-RELOC] application entry verified relocated pointer",
              real="[HELIOS-REAL] result=42", graph="[HELIOS-GRAPH] result=42")
TLS_INIT_OUTPUT = "[HELIOS-TLS] initial=17 write=29 read=29 modify=42 read=42"
TLS_OUTPUT = TLS_INIT_OUTPUT + " tail_initial=0 tail_write=53 tail_read=53 tail_modify=67 tail_read=67"
OUTPUT["tls"] = TLS_OUTPUT
TLS_SHARED_OUTPUT = "[HELIOS-TLS-SHARED] app=1 initial=17 dso=2 initial=31 tail=0 writes=verified distinct=1"
OUTPUT["tls-shared"] = TLS_SHARED_OUTPUT
OUTPUT["mmap"] = "[ANON-MMAP-LIVE] zero=both marker=8 exit=live"


OUTPUT["page-allocator"] = "[PAGE-ALLOC-LIVE] zero=all markers=private allocations=3 pages=6 exit=live"


OUTPUT["malloc"] = OUTPUT["page-allocator"].replace("PAGE-ALLOC", "MALL-ALLOC")


OUTPUT["c-startup"] = "[C-STARTUP] exit-live owners=1 pages=1 exit=60:0"


def one(pattern, serial, start=0, end=None):
    matches = list(re.compile(pattern).finditer(serial, start, len(serial) if end is None else end))
    assert len(matches) == 1, (pattern, len(matches))
    return matches[0]


def validate_orphans(serial, out, result, *, tls=False, tls_shared=False, mmap=False, allocator=False, malloc=False, c_startup=False):
    controls = (*CONTROLS, "tls") if tls else CONTROLS
    if tls_shared:
        controls = (*controls, "tls-shared")
    if mmap:
        controls = (*controls, "mmap")
    if allocator:
        controls = (*controls, "page-allocator")
    if malloc:
        controls = (*controls, "malloc")
    if c_startup:
        controls = (*controls, "c-startup")
    (out / "orphan-lifecycle.log").write_text("\n".join(
        line for line in serial.splitlines()
        if any(tag in line for tag in ("[ORPHAN]", "[ORPHAN-VM]", "[WAIT-LIFE]",
                                       "[WAIT-ALLOC]", "process_resource_cleanup_summary"))
    ) + "\n")
    scenarios = result["parent_before_collection"] = {}
    result["numeric_pid_recycling"] = {
        variant: "not exercisable under the current allocator contract" for variant in controls
    }
    begin = one(r"\[ORPHAN\] begin observer=(\d+:\d+)", serial)
    observer_pid = begin[1].split(":")[0]
    parent_exit = one(r"\[ORPHAN\] parent-exiting parent=((\d+):\d+)", serial)
    parent_vm, parent_pid = parent_exit.groups()
    def allocation(pid):
        return one(rf'\[WAIT-ALLOC\] sequence=(\d+) slot=(\d+) child=({pid}:\d+) parent=(\d+:\d+)', serial)

    parent_alloc = allocation(parent_pid)
    parent, observer = parent_alloc[3], parent_alloc[4]
    assert observer.split(":")[0] == observer_pid
    parent_publication = one(rf"\[WAIT-LIFE\] exit-published parent={observer} child={parent} .*status=Some\(0\)", serial)
    parent_wait = one(rf"\[ORPHAN\] parent-wait target={parent_pid} result=Ok\(0\)", serial)
    assert begin.start() < parent_exit.start() < parent_publication.start() < parent_wait.start()

    def cleanup(pid, identity, relation, slot):
        vm = one(rf"\[ORPHAN-VM\] child={identity} vm=(\d+) regions=0 root_reclaimed=true owned_frames=0 stack_live=false cleanup_pending=false", serial)
        finished = one(rf"\[WAIT-LIFE\] cleanup-completed parent={relation} child={identity} slot={slot} .*state=Reaped", serial)
        counts = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+) swap=(\d+) ipc_entries_cleared=(\d+)", serial)
        assert vm.start() < finished.start() < counts.start()
        assert int(counts[1]) > 0 and int(counts[2]) > 0
        return finished, dict(zip(("frames", "page_tables", "swap", "ipc"), map(int, counts.groups())),
                              vm_generation=int(vm[1]), regions_remaining=0, owned_frames_remaining=0,
                              root_reclaimed=True, kernel_stack_released=True)

    replacements = {}
    for replacement in re.finditer(r"\[ORPHAN\] replacement variant=([\w-]+) child=(\d+)", serial):
        variant, pid = replacement.groups()
        alloc = allocation(pid)
        _, slot, identity, relation = alloc.groups()
        assert relation == observer
        collected = one(rf"\[WAIT-LIFE\] wait-reap parent={observer} child={identity} slot={slot} .*wait=Consumed status=Some\(0\)", serial)
        waited = one(rf"\[ORPHAN\] current target={pid} result=Ok\(0\)", serial)
        assert alloc.start() < replacement.start() < collected.start() < waited.start()
        replacements[pid] = dict(variant=variant, identity=identity, slot=int(slot), waitpid="Ok(0)")
    assert len(replacements) == 3 * len(controls)
    result["replacement_controls"] = list(replacements.values())

    for variant in controls:
        seen = one(rf"\[ORPHAN\] uncollected variant={variant} parent={parent_vm} child=(\d+)", serial)
        pid = seen[1]
        alloc = allocation(pid)
        _, slot, identity, relation = alloc.groups()
        assert relation == parent
        exited = one(rf"\[WAIT-LIFE\] exit-published parent={parent} child={identity} slot={slot} .*wait=Pending status=Some\(0\) retained=true", serial)
        finished, resources = cleanup(pid, identity, parent, slot)
        assert alloc.start() < exited.start() < seen.start() < parent_exit.start()
        if variant == "mmap":
            one(rf"\[ANON-MMAP-SPAN\] pid={pid} requested=8192 effective=8192",
                serial, alloc.start(), exited.start())
        one(re.escape(OUTPUT[variant]), serial, alloc.start(), exited.start())
        assert exited.start() < finished.start()
        assert not re.search(rf"\[WAIT-LIFE\] (?:wait-reap|wake-parent) .*child={identity} ", serial)
        release = one(rf"\[WAIT-LIFE\] slot-release parent={parent} child={identity} slot={slot} .*state=Reaped wait=Pending status=Some\(0\) retained=false next=Some\((\d+)\)", serial)
        occupant = allocation(release[1])
        assert max(finished.start(), parent_publication.start()) < release.start() < occupant.start()
        assert occupant[2] == slot and occupant[3].split(":")[1] != identity.split(":")[1]
        assert "status=None" in serial[occupant.end():serial.index("\n", occupant.end())]
        lower = parent_wait.end()
        for phase in ("after-parent", "after-reuse-batch"):
            wait = one(rf"\[ORPHAN\] {phase} target={pid} result=Err\(Failed\)", serial)
            one(rf"\[WAIT-LIFE\] lookup parent={observer} child={pid} match=None",
                serial, lower, wait.start())
            lower = max(occupant.end(), wait.end())
        scenarios[variant] = dict(status="PASS", parent=parent, child=identity, slot=int(slot),
                                  exit_status=0, disposition="finalized; tombstone discarded on overwrite",
                                  output=OUTPUT[variant],
                                  unrelated_wait="Err(Failed)", after_reuse_wait="Err(Failed)",
                                  resources=resources, occupant=occupant[3], occupant_parent=occupant[4],
                                  fixture_replacement=replacements.get(release[1]))
    _, parent_resources = cleanup(parent_pid, parent, observer, allocation(parent_pid)[2])
    result["orphan_parent_resources"] = parent_resources
    one(r"\[ORPHAN\] done", serial)
