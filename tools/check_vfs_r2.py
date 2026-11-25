#!/usr/bin/env python3
"""Verify exact native R2 identity, ordering and cleanup from serial logs."""

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

    cpu_line, cpu_match = only(r"\[BOOT\] SMP: ([124]) CPU\(s\) reported")
    vcpus = int(cpu_match[1])
    consumed, match = only(
        r"\[VFS-R2\] consumed service=(\d+) child=(\d+) retain=(\d+) epoch=(\d+)"
        r" observer=(\d+) held_receipt=1 binding=0 private=1\b"
    )
    service, child, retain, epoch, observer = map(int, match.groups())
    proof, match = only(
        rf"\[VFS-R2\] proof service={service} incarnation=([1-9]\d*) vm=([1-9]\d*)"
        rf" epoch={epoch} retain={retain} row_retained=false\b"
    )
    incarnation, vm = map(int, match.groups())
    service_reaped, match = only(
        rf"process_reaped pid={service} name='vfs_server' user_frames=([1-9]\d*)"
        r" page_tables=([1-9]\d*) swap_blocks=0\b"
    )
    service_frames, service_tables = map(int, match.groups())
    teardown, match = only(
        rf"teardown child={child} descriptors=(3) frames=([1-9]\d*) tables=([1-9]\d*) swap=0\b"
    )
    descriptors, frames, tables = map(int, match.groups())
    markers = {
        "staged": only(rf"acquire child={child} retain={retain} epoch={epoch} mode=7\b")[0],
        "parked": only(rf"parked parent=1 child={child} retain={retain} epoch={epoch}\b")[0],
        "authenticated": only(
            rf"\[VFS-R2\] authenticated service={service} child={child} retain={retain} epoch={epoch}\b"
        )[0],
        "consumed_checkpoint": consumed,
        "termination": only(rf"process_exit_begin external pid={service} reason=vfs-r2-retained-registration\b")[0],
        "withdrawal": only(
            rf"\[VFS-R2\] withdrawn service={service} child={child} retain={retain} epoch={epoch} release_pending=1\b"
        )[0],
        "proof": proof,
        "service_reaped": service_reaped,
        "rejected": only(rf"\[VFS-R2\] rejected child={child} retain={retain} error=Identity private=1\b")[0],
        "terminal": only(rf"\[VFS-R2\] terminal child={child} authority_retired=1 continuation=0\b")[0],
        "teardown": teardown,
        "heap_drop": only(rf"dropped child={child} process_box=1 fd_table_box=1\b")[0],
        "disposed": only(rf"dispose child={child} retain={retain} owner=reclaimed\b")[0],
        "observer_wait": only(r"\[VFS-R2\] init observed creation failure and reaped observer status=0\b")[0],
    }
    if cpu_line >= markers["staged"] or list(markers.values()) != sorted(set(markers.values())):
        raise ValueError(f"{path}: unexpected event order: {markers}")
    settled = only(rf"settled child={child} retain={retain} epoch={epoch}\b")[0]
    observer_exit = only(rf"process_mark_finished pid={observer} name='true' reason=exit code=0\b")[0]
    if not teardown < settled or not markers["parked"] < observer_exit < markers["observer_wait"]:
        raise ValueError(f"{path}: missing ordered PID settlement or independent observer completion")
    forbidden = (
        rf"\[VFS-L1C-2\] (binding|committed|enqueue|first dispatch selected) child={child}\b"
        r"|KERNEL PANIC|panicked at"
    )
    if any(re.search(forbidden, line, re.IGNORECASE) for line in lines):
        raise ValueError(f"{path}: unexpected publication or panic")
    return dict(
        serial=path.name, vcpus=vcpus, service=service, child=child, retain=retain, epoch=epoch,
        incarnation=incarnation, vm_generation=vm, observer=observer,
        service_frames=service_frames, service_page_tables=service_tables,
        descriptors=descriptors, frames=frames, page_tables=tables,
        lines=dict(markers, settled=settled, observer_exit=observer_exit),
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("serial", nargs="+", type=Path)
    args = parser.parse_args()
    print(json.dumps([check(path) for path in args.serial], indent=2))
