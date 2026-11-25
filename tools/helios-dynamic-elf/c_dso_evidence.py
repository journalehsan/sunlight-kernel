"""Verify utility relocation, call results, rollback and guest teardown."""
import re

from orphan_evidence import one


def validate_c_dso(serial, out, result):
    cases = {
        "import": "unsupported utility import",
        "needed": "utility DT_NEEDED",
        "relocation": "unsupported relocation",
        "lookup": "missing library export",
        "after_mapping": "utility setup after mapping",
        "after_resolution": "utility setup after resolution",
        "after_publication": "initial stack",
    }
    names = {"helios_" + name for name in ("memset", "memcpy", "strlen", "write", "exit")}
    rollback = {}
    lower = 0
    for case, stage in cases.items():
        row = one(rf"\[C-DSO\] rollback verified case=c_dso_{case} "
                  r"pid=(\d+) free=(\d+) tables=(\d+) owned=0", serial)
        pid, free, tables = map(int, row.groups())
        mapped = one(rf"\[HELIOS-SHARED\] pid={pid} source=embedded-bundle library=Program/libhelios-util\.so",
                     serial, lower if lower else serial.rfind("[HELIOS-HANDOFF] interpreter=", 0, row.start()),
                     row.start())
        one(rf"\[HELIOS-HANDOFF\] rejected stage={stage} pid={pid}\b",
            serial, mapped.end(), row.start())
        reclaimed = one(rf"\[HELIOS-HANDOFF\] rollback pid={pid} frames=(\d+) tables=(\d+)",
                        serial, mapped.end(), row.start())
        staged = serial[mapped.end():row.start()]
        assert int(reclaimed[1]) > 0 and int(reclaimed[2]) > 0
        assert not re.search(rf"\[WAIT-LIFE\] published .*child={pid}:", staged)
        slots = re.findall(rf"\[C-DSO-RELOC\] pid={pid} .* symbol=(\w+)", staged)
        assert sorted(slots) == (sorted(names) if case == "after_publication" else [])
        rollback[case] = dict(pid=pid, stage=stage, free=free, tables=tables,
                              published_slots=len(slots), owned=0, regions=0,
                              reclaimed_frames=int(reclaimed[1]), reclaimed_tables=int(reclaimed[2]))
        lower = row.end()
    assert len({(row["pid"], row["free"], row["tables"]) for row in rollback.values()}) == 1

    children = []
    lower = one(r"\[C-STARTUP-WAIT\] mode=L .*again=Err\(Failed\)", serial).end()
    for attempt in (1, 2):
        wait = one(rf"\[C-DSO-WAIT\] attempt={attempt} pid=(\d+) waitpid=Ok\(0\) again=Err\(Failed\)", serial)
        pid = wait[1]
        exited = one(rf"process_mark_finished pid={pid} .*code=0\b", serial)
        vm = one(rf"\[ORPHAN-VM\] child={pid}:\d+ vm=(\d+) regions=0 root_reclaimed=true "
                 r"owned_frames=0 stack_live=false cleanup_pending=false", serial)
        counts = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
        assert lower < exited.start() < wait.start() and exited.start() < vm.start() < counts.start()
        assert int(counts[1]) > 0 and int(counts[2]) > 0
        slots = re.findall(rf"\[C-DSO-RELOC\] pid={pid} hash=verified type=R_X86_64_JUMP_SLOT "
                           r"symbol=(\w+) slot=(0x\w+) target=(0x\w+)", serial[lower:exited.start()])
        assert {symbol for symbol, _, _ in slots} == names and len(slots) == len(names)
        assert len({slot for _, slot, _ in slots}) == len(names)
        for marker in (
            "[C-DSO] _start argc=2 argv=verified envc=0 auxv=verified rsp_mod16=0 registers=match",
            "[C-DSO] buffer=slot-0 generation=1 bytes=37 src-offset=1 dst-offset=18 memset=0 "
            "memcpy=0 strlen=16 fault=-14 overflow=-75 unterminated=-61 overlap=-22 output-fault=-9 zero=checked",
            "HELIOS-C-DSO-OK",
            "[C-DSO] free=0 stale=-22 owners=0 pages=0 exit=60:0",
        ):
            lower = one(re.escape(marker), serial, lower, exited.start()).end()
        children.append(dict(pid=int(pid), vm_generation=int(vm[1]), frames=int(counts[1]),
                             page_tables=int(counts[2]), regions=0, owned=0, status=0,
                             waited=0, repeated_wait="Failed", output="HELIOS-C-DSO-OK",
                             buffer=dict(slot=0, generation=1, bytes=37, source=1, destination=18),
                             relocations=[dict(symbol=symbol, slot=slot, target=target)
                                          for symbol, slot, target in slots]))
        lower = wait.end()
    assert len({row["vm_generation"] for row in children}) == 2
    assert len({(row["frames"], row["page_tables"]) for row in children}) == 1
    result["c_dso"] = dict(children=children, rollback=rollback)
    (out / "c-dso.log").write_text("\n".join(
        line for line in serial.splitlines()
        if "[C-DSO" in line or "HELIOS-C-DSO-OK" in line
    ) + "\n")
