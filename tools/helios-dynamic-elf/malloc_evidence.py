"""Check byte requests, shim-specific rejection and failure teardown."""
import re

from orphan_evidence import one


def validate_malloc(serial, out, result):
    limits = result["malloc_limits"]
    limits.update(minimum_bytes=1, maximum_bytes=32768, alignment=4096,
                  total_backing_bytes=65536, zero_fill="all new backing")
    result["malloc_api"] = dict(
        allocation="helios_malloc returns a checked handle through its output argument",
        resolve="helios_resolve returns the live guest address or zero",
        free="helios_free accepts the complete handle",
        errors={"invalid": -22, "exhausted": -12, "publication": -5},
    )
    rows = {}
    for owner, slot, generation, size, alignment, status in re.findall(
        r"(?m)^\[BOUNDED-MALLOC\] ([a-f0-9]{16}) ([a-f0-9]{16}) ([a-f0-9]{16}) "
        r"([a-f0-9]{16}) ([a-f0-9]{16}) ([a-f0-9]{16})$", serial,
    ):
        owner, slot, generation, size, alignment, status = (
            int(word, 16) for word in (owner, slot, generation, size, alignment, status)
        )
        assert alignment == limits["alignment"]
        if status >= 1 << 63:
            status -= 1 << 64
        rows.setdefault(owner, []).append(dict(slot=slot, generation=generation, size=size,
                                               alignment=alignment, status=status))
    scenarios = list(result["malloc_children"].values())
    scenarios.append(result["parent_before_collection"]["malloc"])
    scenarios.extend(row for row in result["replacement_controls"] if row["variant"] == "malloc")
    for child in scenarios:
        identity = child.get("child") or child["identity"]
        requests = rows.pop(int(identity.split(":")[0]))
        allocations = [row for row in child["operations"] if row["op"] == "A"]
        assert len(allocations) == len(requests)
        for allocation, request in zip(allocations, requests):
            assert request["status"] == allocation["status"]
            if not request["status"]:
                assert request["slot"] == allocation["slot"]
                assert request["generation"] == allocation["generation"]
                assert 1 <= request["size"] <= limits["maximum_bytes"]
                assert (request["size"] + 4095) // 4096 == allocation["pages"]
        child["requests"] = requests

    mmap = result["anonymous_mmap"][0]
    pie = result["variants"]["pie"]
    checks = {
        "B": (0, mmap["frames"], mmap["tables"], (
            "misaligned=-22", "generation=-22", "size=-22",
            "shim-publication=-5 unpublished=-22 owners=1 pages=2 marker=71",
            "done owners=0 pages=0")),
        "J": (43, mmap["frames"] + 2, mmap["tables"] + 2, (
            "child-setup-aborted live=2 marker=4a exit=43",)),
        "I": (44, pie["user_frames_reclaimed"] + 2, pie["page_tables_reclaimed"] + 2, (
            "interpreter-setup-aborted live=2 marker=49 exit=44",)),
        "Y": (0, mmap["frames"], mmap["tables"], (
            "foreign-child-token=-22 owners=1 pages=1 marker=59",)),
    }
    prior = one(r"\[MALL-ALLOC-WAIT\] marker=c .*", serial).end()
    for marker, (status, frames, tables, messages) in checks.items():
        wait = one(rf"\[MALL-ALLOC-WAIT\] marker={marker} pid=(\d+) waitpid=Ok\({status}\) again=Err\(Failed\)", serial)
        pid = wait[1]
        allocated = one(
            rf"\[WAIT-ALLOC\] sequence=\d+ slot=(\d+) child=({pid}:\d+) parent=(\d+:\d+)",
            serial, prior, wait.start(),
        )
        slot, identity, parent = allocated.groups()
        exited = one(rf"\[WAIT-LIFE\] exit-published parent={parent} child={identity} slot={slot} .*status=Some\({status}\)", serial)
        one(rf"\[WAIT-LIFE\] wait-reap parent={parent} child={identity} slot={slot} .*wait=Consumed status=Some\({status}\)", serial,
            exited.end(), wait.start())
        one(rf"\[ORPHAN-VM\] child={identity} vm=\d+ regions=0 root_reclaimed=true owned_frames=0 stack_live=false cleanup_pending=false", serial)
        one(rf"process_resource_cleanup_summary pid={pid} frames={frames} pts={tables} ", serial)
        for message in messages:
            one(r"\[MALLOC-CHECK\] " + re.escape(message), serial, prior, exited.start())
        requests = rows.pop(int(pid), [])
        if marker == "B":
            assert [(row["size"], row["status"]) for row in requests] == [(4115, 0), (19, -5)]
            transitions = re.findall(
                rf"\[ANON-MMAP-VM\] pid={pid} op=(\w+) ok=true before=(.*) after=(.*)", serial)
            assert [row[0] for row in transitions] == ["map", "map", "unmap", "unmap"]
            _, before, after = transitions[1]
            assert before == transitions[2][2]
            assert int(after.split(",")[0][1:]) - int(transitions[2][1].split(",")[0][1:]) == 1
        if marker == "I":
            mapping = one(r"\[SYSCALL\] mmap\(0x0, 0x2000\) -> (0x[a-f0-9]+)",
                          serial, allocated.end(), exited.start())
            base = int(mapping[1], 16)
            assert base and base % limits["alignment"] == 0
            assert not requests
            requests.append(dict(slot=0, generation=1, size=4097, alignment=limits["alignment"],
                                 guest_base=base, status=0))
        if marker == "Y":
            owner = one(r" owner=(\d+)$", serial, wait.end(), serial.index("\n", wait.end()))[1]
            producer = one(
                rf"\[WAIT-ALLOC\] sequence=\d+ slot=(\d+) child=({owner}:\d+) parent={parent}",
                serial, prior, exited.start(),
            )
            producer_slot, producer_identity = producer.groups()
            one(rf"\[WAIT-LIFE\] wait-reap parent={parent} child={producer_identity} slot={producer_slot} .*wait=Consumed status=Some\(0\)",
                serial, producer.end(), wait.start())
            one(rf"\[ORPHAN-VM\] child={producer_identity} vm=\d+ regions=0 root_reclaimed=true owned_frames=0 stack_live=false cleanup_pending=false",
                serial)
            one(rf"process_resource_cleanup_summary pid={owner} frames={frames} pts={tables} ", serial)
            rejected = one(rf"(?m)^\[MALL-ALLOC\] F {int(pid):016x} {int(owner):016x} "
                           r"0000000000000000 0000000000000001 ([a-f0-9]{16}) "
                           r"0000000000000001 0000000000000001 0000000000000001 "
                           r"0000000000000059 ffffffffffffffea$", serial)
            result["malloc_foreign_token"] = dict(
                producer=producer_identity, consumer=identity, slot=0, generation=1,
                guest_base=int(rejected[1], 16), requested=37, free_status=-22)
            assert [(row["size"], row["status"]) for row in requests] == [(37, 0)]
        result["malloc_children"][marker] = dict(
            child=identity, parent=parent, slot=int(slot), exit_status=status, waitpid=f"Ok({status})",
            frames=frames, page_tables=tables, requests=requests, checks=messages)
        prior = wait.end()
    assert not rows, rows
    setup_pids = {result["malloc_children"][marker]["child"].split(":")[0] for marker in checks}
    setup_pids.add(result["malloc_foreign_token"]["producer"].split(":")[0])
    transitions = re.compile(r"^\[ANON-MMAP-VM\] pid=(\d+) ")
    with (out / "malloc.log").open("a") as log:
        for line in serial.splitlines():
            transition = transitions.match(line)
            if transition and transition[1] in setup_pids:
                print(line, file=log)
    log = (out / "host-bounded-malloc.log").read_text()
    counts = one(r"(?m)^(\d+) passed; 0 failed$", log)
    result["host_tests_passed"]["malloc_shim"] = int(counts[1])
