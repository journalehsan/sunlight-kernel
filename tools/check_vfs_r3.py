#!/usr/bin/env python3
"""Check ordering and ownership for a native R3 service retirement witness."""

import argparse
import json
from pathlib import Path
import re


def check(path):
    lines = path.read_text(errors="replace").splitlines()

    def only(pattern):
        matches = [(n, m) for n, line in enumerate(lines, 1) if (m := re.search(pattern, line))]
        if len(matches) != 1:
            raise ValueError(f"{path}: expected one {pattern!r}, found {len(matches)}")
        return matches[0]

    cpu_line, cpu = only(r"\[BOOT\] SMP: ([124]) CPU\(s\) reported")
    retained, match = only(
        r"\[VFS-R3\] retained service=(\d+) child=(\d+) retain=(\d+) epoch=(\d+)"
        r" incarnation=([1-9]\d*) vm=([1-9]\d*)\b"
    )
    service, child, retain, epoch, child_inc, child_vm = map(int, match.groups())
    proof, match = only(
        rf"\[VFS-R3\] proof service={service} incarnation=([1-9]\d*) vm=([1-9]\d*)"
        rf" epoch={epoch} retain={retain} row_retained=false\b"
    )
    service_inc, service_vm = map(int, match.groups())

    def cleanup(pid, name):
        line, counts = only(
            rf"process_reaped pid={pid} name='{name}' user_frames=([1-9]\d*)"
            r" page_tables=([1-9]\d*) swap_blocks=(0)\b"
        )
        frames, tables, swap = map(int, counts.groups())
        return line, dict(frames=frames, page_tables=tables, swap_blocks=swap)

    service_reap, service_resources = cleanup(service, "vfs_server")
    child_reap, child_resources = cleanup(child, "vfs-l1c-private")
    markers = {
        "staged": only(rf"acquire child={child} retain={retain} epoch={epoch} mode=8\b")[0],
        "parked": only(rf"parked parent=1 child={child} retain={retain} epoch={epoch}\b")[0],
        "retained": retained,
    }
    for event in ("binding", "committed", "enqueue", "first dispatch selected", "metadata completed"):
        markers[event] = only(rf"\[VFS-L1C-2\] {event} child={child}\b")[0]
    markers.update(
        stat=only(r"\[VFS-R3\] userspace stat /tmp succeeded")[0],
        checkpoint=only(rf"\[VFS-R3\] checkpoint child={child} stat=complete\b")[0],
        termination=only(
            rf"process_exit_begin external pid={service} reason=vfs-r3-published-child\b"
        )[0],
        withdrawal=only(
            rf"\[VFS-R3\] withdrawn service={service} child={child} retain={retain} epoch={epoch}"
            r" release_pending=1 continuation=0\b"
        )[0],
        proof=proof,
        service_reap=service_reap,
        continued=only(
            rf"\[VFS-R3\] continued child={child} retain={retain}"
            r" continuation=0 private=0 scheduler=1 replay=Identity\b"
        )[0],
        namespace=only(r"\[VFS-R3\] userspace stat=Again chdir=Again getcwd=unchanged\b")[0],
        child_exit=only(rf"process_mark_finished pid={child} name='vfs-l1c-private' reason=exit code=0\b")[0],
    )
    wait = only(r"\[VFS-R3\] init consumed child status=0 once\b")[0]
    markers.update(
        sorted({"child_reap": child_reap, "wait": wait}.items(), key=lambda event: event[1])
    )
    markers.update(
        checked=only(
            rf"\[VFS-R3\] reaped child={child} wait=consumed pid_reserved=0 root=freed"
            r" stack=freed owned_frames=0 fd_slots=0\b"
        )[0],
        disposed=only(rf"\[VFS-R3\] disposed child={child} retain={retain} owner=scheduler cancel=Stale\b")[0],
        observer_wait=only(r"\[VFS-R3\] init reaped observer status=0\b")[0],
    )
    if cpu_line >= markers["staged"] or list(markers.values()) != sorted(set(markers.values())):
        raise ValueError(f"{path}: unexpected event order: {markers}")
    if any(re.search(rf"KERNEL PANIC|panicked at|teardown child={child}\b", line, re.IGNORECASE)
           for line in lines):
        raise ValueError(f"{path}: panic or private rollback observed")
    return dict(
        serial=path.name, vcpus=int(cpu[1]), service=service, child=child, retain=retain, epoch=epoch,
        service_incarnation=service_inc, service_vm_generation=service_vm,
        child_incarnation=child_inc, child_vm_generation=child_vm,
        service_resources=service_resources, child_resources=child_resources, lines=markers,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("serial", nargs="+", type=Path)
    print(json.dumps([check(path) for path in parser.parse_args().serial], indent=2))
