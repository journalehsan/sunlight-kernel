"""Correlate C startup results with guest wait and address-space teardown."""
import re

from orphan_evidence import one


def validate_startup(serial, out, result):
    rollbacks = {}
    for case in ("overflow", "before_entry", "argc", "argv", "string", "envp", "auxv", "alignment"):
        row = one(rf"\[C-STARTUP\] rollback verified case=c_startup_{case} "
                  r"pid=(\d+) free=(\d+) tables=(\d+) owned=0", serial)
        rollbacks[case] = dict(pid=int(row[1]), free=int(row[2]), tables=int(row[3]), owned=0)
    assert all(row == rollbacks["overflow"] for row in rollbacks.values())
    one(r"\[C-STARTUP\] pre-entry stack validated argc=2 envc=0 context=unpublished", serial)

    marker = {
        "P": "output=HELIOS-C-STARTUP-OK write=1 munmap=11 free=0 stale=-22 owners=0 pages=0 exit=60:0",
        "N": "negatives=argc,argv-null,argv-pointer,argv-unterminated,envp-null,auxv-tag,auxv-value,"
             "auxv-null,alignment,overflow,truncated",
        "A": "allocation-init=-12 owners=0 pages=0 unpublished=1",
        "W": "write=1 fault=-14 exit-live owners=1 pages=1 exit=60:44",
        "L": "exit-live owners=1 pages=1 exit=60:0",
    }

    def cleanup(pid, status):
        exited = one(rf"process_mark_finished pid={pid} .*code={status}\b", serial)
        vm = one(rf"\[ORPHAN-VM\] child={pid}:\d+ vm=(\d+) regions=0 root_reclaimed=true "
                 r"owned_frames=0 stack_live=false cleanup_pending=false", serial)
        counts = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
        assert exited.start() < vm.start() < counts.start()
        assert int(counts[1]) > 0 and int(counts[2]) > 0
        return exited.start(), dict(frames=int(counts[1]), page_tables=int(counts[2]),
                                    vm_generation=int(vm[1]), owned_frames=0, regions=0)

    children = {}
    lower = one(r"\[ORPHAN\] done", serial).end()
    for mode, status in (("P", 0), ("N", 42), ("A", 43), ("W", 44), ("L", 0)):
        waited = one(rf"\[C-STARTUP-WAIT\] mode={mode} pid=(\d+) waitpid=Ok\({status}\) "
                     r"again=Err\(Failed\)", serial)
        pid = waited[1]
        exited, resources = cleanup(pid, status)
        values = one(rf"\[C-STARTUP-VALUES\] pid={pid} mode={mode} "
                     r"stack_bytes,argv0_offset,argv1_offset,auxv_values=([,0-9]+)", serial, lower, exited)
        decoded = list(map(int, values[1].split(",")))
        assert len(decoded) == 16
        assert decoded[:3] == [288, 262, 260] and decoded[-3:] == [262, 272, 0]
        assert lower < exited < waited.start()
        one(re.escape("[C-STARTUP] " + marker[mode]), serial, lower, exited)
        one(r"\[C-STARTUP\] argc=2 .* envc=0 .* rsp_mod16=0 argument_registers=match strings=guest-owned",
            serial, lower, exited)
        if mode in "PLW":
            one(r"\[C-STARTUP\] getpid=39 mmap=9 malloc=37 alignment=4096 zero=4096 markers=31,72 "
                r"slot=0 generation=1", serial, lower, exited)
        children[mode] = dict(pid=int(pid), status=status, stack_and_auxv=decoded, **resources)
        lower = waited.end()

    replacements = [row for row in result["replacement_controls"] if row["variant"] == "c-startup"]
    assert len(replacements) == 3
    for row in replacements:
        pid = row["identity"].split(":")[0]
        allocated = one(rf"\[WAIT-ALLOC\] .* child={row['identity']} ", serial)
        exited, resources = cleanup(pid, 0)
        # Each replacement writes the success marker and starts at generation one.
        one(rf"\[C-STARTUP-VALUES\] pid={pid} mode=P .+", serial, allocated.end(), exited)
        row.update(resources)
    assert serial.count("[C-STARTUP] " + marker["P"]) == 4
    assert serial.count("[C-STARTUP] getpid=39 mmap=9 malloc=37") == 7

    baseline = children["P"]
    orphan = result["parent_before_collection"]["c-startup"]["resources"]
    for row in (children["N"], children["A"], *replacements):
        assert (row["frames"], row["page_tables"]) == (baseline["frames"], baseline["page_tables"])
    for row in (children["W"], children["L"], orphan):
        assert row["frames"] == baseline["frames"] + 1
        assert row["page_tables"] > baseline["page_tables"]

    log = (out / "host-c-startup.log").read_text()
    count = re.search(r"^(\d+) tests passed$", log, re.MULTILINE)
    assert count and int(count[1]) == 16
    result["host_tests_passed"]["c_startup"] = int(count[1])
    result["c_startup"] = dict(rollback=rollbacks, children=children, replacements=replacements)

    pids = [str(row["pid"]) for row in children.values()]
    pids.extend(row["identity"].split(":")[0] for row in replacements)
    pids.append(result["parent_before_collection"]["c-startup"]["child"].split(":")[0])
    lifecycle = re.compile(r"(?:pid=|child=)(?:" + "|".join(pids) + r")\b")
    (out / "c-startup.log").write_text("\n".join(
        line for line in serial.splitlines()
        if "[C-STARTUP" in line or lifecycle.search(line)
    ) + "\n")
