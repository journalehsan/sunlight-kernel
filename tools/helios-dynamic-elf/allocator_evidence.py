"""Validate allocator operations and correlate guest identities with kernel reclamation."""
import re
import subprocess

from orphan_evidence import one


def run_host_allocator(out, suite="page-allocator"):
    binary = out / f"{suite}-tests"
    command = ["gcc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
               f"tools/helios-dynamic-elf/{suite}-test.c", "-o", str(binary)]
    if suite == "c-startup":
        command[4:4] = ["-DHELIOS_STARTUP_HOST_TEST", "-D_start=fixture_start",
                        "tools/helios-dynamic-elf/c-startup-entry.S"]
    with (out / f"host-{suite}.log").open("w") as log:
        print(" ".join(command), file=log, flush=True)
        subprocess.run(command, stdout=log, stderr=log, check=True)
        subprocess.run([str(binary)], stdout=log, stderr=log, check=True)
    binary.unlink()


def operations(serial, pid, marker, expected_live, tag):
    keys = ("owner", "handle_owner", "slot", "generation", "base", "pages",
            "used", "total_pages", "marker", "status")
    raw = re.findall(
        rf"^\[{tag}\] ([A-Z!]) ({int(pid):016x}(?: [a-f0-9]{{16}}){{9}})$",
        serial, re.MULTILINE,
    )
    rows = []
    live = {}
    generation = 0
    for op, words in raw:
        row = dict(zip(keys, (int(word, 16) for word in words.split())))
        row["op"] = op
        if row["status"] >= 1 << 63:
            row["status"] -= 1 << 64
        slot, pages, base = row["slot"], row["pages"], row["base"]
        handle = tuple(row[key] for key in ("handle_owner", "generation", "base", "pages"))
        if not rows:
            assert op == "I" and row["marker"] == ord(marker)
            assert not any(handle) and not slot and not row["status"]
        elif op == "A":
            if row["status"] == 0:
                generation += 1
                assert row["generation"] == generation and row["handle_owner"] == int(pid)
                assert 1 <= pages <= 8 and 0 <= slot < 8 and base and base % 4096 == 0
                assert slot not in live
                for _, _, other_base, other_pages in live.values():
                    assert (base + pages * 4096 <= other_base or
                            other_base + other_pages * 4096 <= base)
                live[slot] = handle
            else:
                assert row["status"] in (-5, -12, -22)
                assert not (slot or base or row["handle_owner"] or row["generation"])
        else:
            assert op == "F"
            valid = live.get(slot) == handle
            if row["status"] == 0:
                assert valid
                del live[slot]
            elif row["status"] == -12:
                assert valid
            else:
                assert row["status"] == -22 and not valid
        assert row["used"] == len(live) <= 8
        assert row["total_pages"] == sum(entry[3] for entry in live.values()) <= 16
        rows.append(row)
    assert rows and len(live) == expected_live
    return rows


def verify_live(serial, pid, marker, baseline, tag):
    rows = operations(serial, pid, marker, 3, tag)
    assert [row["op"] for row in rows] == ["I", "A", "A", "A"]
    assert [row["pages"] for row in rows[1:]] == [1, 2, 3]
    assert [row["marker"] for row in rows[1:]] == list(
        range(0x80 + 3 * (ord(marker) - ord("a")), 0x83 + 3 * (ord(marker) - ord("a")))
    )
    counts = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
    assert int(counts[1]) == baseline["frames"] + 6
    assert int(counts[2]) == baseline["tables"] + 2
    return rows


def validate_allocator(serial, out, result, *, malloc=False):
    tag = "MALL-ALLOC" if malloc else "PAGE-ALLOC"
    variant = "malloc" if malloc else "page-allocator"
    key = "malloc" if malloc else "page_allocator"
    fixture = one(r"(?m)^([a-f0-9]{64})  (.+/elf-mmap\.sunapp/Program/probe)$",
                  (out / "SHA256SUMS").read_text())
    command = one(r"(?m)^\+ (gcc .* -DPAGE_ALLOCATOR_FIXTURE .*)$",
                  (out / "build.log").read_text())
    result[f"{key}_fixture"] = dict(sha256=fixture[1], artifact=fixture[2], command=command[1])
    result[f"{key}_limits"] = dict(page_bytes=4096, slots=8, single_pages=8, total_pages=16)
    baseline = result["anonymous_mmap"][0]
    prior = one(r"\[ANON-MMAP-FAULT-WAIT\] marker=Z .*", serial).end()
    children = {}
    for marker in "AFTLQEabc":
        wait = one(rf"\[{tag}-WAIT\] marker={marker} pid=(\d+) waitpid=Ok\(0\) again=Err\(Failed\)", serial)
        pid = wait[1]
        slot, identity, parent = one(
            rf"\[WAIT-ALLOC\] sequence=\d+ slot=(\d+) child=({pid}:\d+) parent=(\d+:\d+)", serial,
            prior, wait.start(),
        ).groups()
        exited = one(rf"\[WAIT-LIFE\] exit-published parent={parent} child={identity} slot={slot} .*status=Some\(0\)", serial)
        reaped = one(rf"\[WAIT-LIFE\] wait-reap parent={parent} child={identity} slot={slot} .*wait=Consumed status=Some\(0\)", serial)
        assert prior < exited.start() < reaped.start() < wait.start()
        one(rf"\[ORPHAN-VM\] child={identity} vm=\d+ regions=0 root_reclaimed=true owned_frames=0 stack_live=false cleanup_pending=false", serial)
        cleanup = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
        frames, tables = map(int, cleanup.groups())
        window = serial[prior:exited.start()]
        traces = re.findall(rf"\[ANON-MMAP-VM\] pid={pid} op=(\w+) ok=(\w+) before=(.*) after=(.*)", window)
        for _, ok, before, after in traces:
            if ok == "false":
                assert before == after
        if marker.islower():
            rows = verify_live(serial, pid, marker, baseline, tag)
            one(rf"\[{tag}-LIVE\] zero=all markers=private allocations=3 pages=6 exit=live", window)
        else:
            rows = operations(window, pid, marker, 0, tag)
            one(rf"\[{tag}-DONE\] owners=0 pages=0", window)
            assert (frames, tables) == (baseline["frames"], baseline["tables"])
        failures = [(row["op"], row["status"], row["used"], row["total_pages"])
                    for row in rows if row["status"]]
        if marker == "A":
            assert failures == [
                ("F", -22, 3, 6), ("F", -22, 3, 6), ("F", -22, 2, 4), ("F", -22, 3, 6),
                ("A", -22, 1, 1), ("A", -22, 1, 1), ("A", -22, 1, 1),
                ("A", -12, 3, 16), ("A", -12, 8, 8),
            ]
            assert [op for op, _, _, _ in traces].count("unmap") == sum(
                row["op"] == "F" and row["status"] == 0 for row in rows)
            assert all(ok == "true" for _, ok, _, _ in traces)
            assert all(after.endswith("None, None)") for op, _, _, after in traces if op == "unmap")
            rejected = [row for row in rows if row["op"] == "F" and row["status"] == -22]
            foreign, interior, duplicate, stale = rejected
            assert foreign["handle_owner"] != foreign["owner"]
            assert interior["owner"] == interior["handle_owner"]
            assert interior["base"] == duplicate["base"] + 4096
            assert stale["slot"] == duplicate["slot"] and stale["generation"] == duplicate["generation"]
        elif marker in "FTL":
            fault = {"F": "frame page=2", "T": "page-table page=1",
                     "L": "mapping-publication leaves=3"}[marker]
            one(rf"\[PAGE-ALLOC-VM\] pid={pid} fault={fault}", window)
            one(rf"\[PAGE-ALLOC-VM\] pid={pid} rollback=exact free=\d+ owned=\d+ regions=\d+ cursor=unchanged", window)
            assert failures == [("A", -22 if marker == "L" else -12, 1, 1), ("F", -22, 1, 1)]
            assert sum(op == "map" and ok == "false" for op, ok, _, _ in traces) == 1
        elif marker == "Q":
            assert failures == [("A", -5, 1, 1), ("F", -22, 1, 1)]
            _, mapping, rollback, *_ = traces
            assert mapping[:2] == ("map", "true") and rollback[:2] == ("unmap", "true")
            assert mapping[2] == rollback[3]
            assert int(rollback[3].split(",")[0][1:]) - int(rollback[2].split(",")[0][1:]) >= 2
        elif marker == "E":
            assert failures == [("A", -12, 3, 3), ("F", -12, 3, 3)]
            assert [(op, before.split(", ")[1]) for op, ok, before, _ in traces if ok == "false"] == [
                ("map", "256"), ("map", "256"), ("unmap", "256")]
        else:
            assert not failures
        children[marker] = dict(child=identity, parent=parent, slot=int(slot),
                                waitpid="Ok(0)", frames=frames, page_tables=tables, operations=rows)
        prior = wait.end()
    assert len({row["child"] for row in children.values()}) == len(children)
    assert len({children[marker]["slot"] for marker in "abc"}) == 1
    result[f"{key}_children"] = children
    orphan = result["parent_before_collection"][variant]
    orphan["operations"] = verify_live(serial, orphan["child"].split(":")[0], "d", baseline, tag)
    replacements = [row for row in result["replacement_controls"] if row["variant"] == variant]
    assert len(replacements) == 3
    for marker, replacement in zip("efg", replacements):
        identity = replacement["identity"]
        one(rf"\[ORPHAN-VM\] child={identity} vm=\d+ regions=0 root_reclaimed=true owned_frames=0 stack_live=false cleanup_pending=false", serial)
        replacement["operations"] = verify_live(serial, identity.split(":")[0], marker, baseline, tag)
    allocator_pids = {row["child"].split(":")[0] for row in children.values()}
    allocator_pids.add(orphan["child"].split(":")[0])
    allocator_pids.update(row["identity"].split(":")[0] for row in replacements)
    transitions = re.compile(r"^\[ANON-MMAP-VM\] pid=(\d+) ")
    (out / f"{variant}.log").write_text("\n".join(
        line for line in serial.splitlines()
        if line.startswith(f"[{tag}") or (malloc and line.startswith(("[BOUNDED-MALLOC]", "[MALLOC-CHECK]"))) or
        ((transition := transitions.match(line)) and transition[1] in allocator_pids)
    ) + "\n")
    if malloc:
        return
    suites = {}
    for name, file in (("kernel", "host-mm2a.log"), ("elf", "host-elf.log"),
                       ("pid_allocator", "allocator.log"), ("page_allocator", "host-page-allocator.log")):
        log = (out / file).read_text()
        counts = re.findall(r"(\d+) passed; (\d+) failed", log)
        assert counts and all(failed == "0" for _, failed in counts)
        suites[name] = sum(int(passed) for passed, _ in counts)
    result["host_tests_passed"] = suites
