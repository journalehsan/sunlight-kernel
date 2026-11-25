#!/usr/bin/env python3
"""Check one pending-child retirement transaction per fresh guest."""

import argparse
import json
import re
from pathlib import Path


def check(path, before_acquisition=False):
    lines = path.read_text(errors="replace").splitlines()

    def only(pattern):
        matches = [
            (number, match)
            for number, line in enumerate(lines, 1)
            if (match := re.search(pattern, line))
        ]
        if len(matches) != 1:
            raise ValueError(f"{path}: expected one {pattern!r}, found {len(matches)}")
        return matches[0]

    if before_acquisition:
        checkpoint_pattern = (
            r"\[VFS-R1B\] checkpoint service=(\d+) child=(\d+) retain=(\d+) epoch=(\d+)"
            r" unsent=1 delivered=0 preparing=1 caller_reserved=1\b"
        )
        mode, reason, discarded = 6, "vfs-r1b-unsent-registration", 0
    else:
        checkpoint_pattern = r"\[VFS-R1\] B held service=(\d+) child=(\d+) retain=(\d+) epoch=(\d+)"
        mode, reason, discarded = 5, "vfs-r1-held-registration", 1
    checkpoint, match = only(checkpoint_pattern)
    service, child, retain, epoch = map(int, match.groups())
    cancelled, match = only(rf"cancelled mode={mode} child={child} other=(\d+) reclaimed=1\b")
    observer = int(match[1])
    teardown, match = only(
        rf"teardown child={child} descriptors=(3) frames=([1-9]\d*) tables=([1-9]\d*) swap=(0)\b"
    )
    descriptors, frames, tables, swap = map(int, match.groups())
    proof, match = only(
        rf"\[VFS-R1\] proof service={service} incarnation=([1-9]\d*) vm=([1-9]\d*) "
        rf"epoch={epoch} retain={retain} row_retained=false\b"
    )
    incarnation, vm = map(int, match.groups())
    service_reaped, match = only(
        rf"process_reaped pid={service} name='vfs_server' user_frames=([1-9]\d*) "
        r"page_tables=([1-9]\d*) swap_blocks=(0)\b"
    )
    service_frames, service_tables, service_swap = map(int, match.groups())
    markers = {
        "staged": only(rf"acquire child={child} retain={retain} epoch={epoch} mode={mode}\b")[0],
        "parked": only(rf"parked parent=1 child={child} retain={retain} epoch={epoch}\b")[0],
        "checkpoint" if before_acquisition else "held": checkpoint,
        "termination": only(rf"process_exit_begin external pid={service} reason={reason}\b")[0],
        "proof": proof,
        "service_reaped": service_reaped,
        "terminal": only(rf"\[VFS-R1\] terminal child={child} authority_retired=1 continuation=0 held_copy_discarded={discarded}\b")[0],
        "teardown": teardown,
        "heap_drop": only(rf"dropped child={child} process_box=1 fd_table_box=1\b")[0],
        "cancelled": cancelled,
        "disposed": only(rf"dispose child={child} retain={retain} owner=reclaimed\b")[0],
        "settled": only(rf"settled child={child} retain={retain} epoch={epoch}\b")[0],
    }
    if list(markers.values()) != sorted(set(markers.values())):
        raise ValueError(f"{path}: unexpected event order: {markers}")
    observer_exit = only(rf"process_mark_finished pid={observer} name='true' reason=exit code=0\b")[0]
    observer_wait = only(r"\[VFS-R1\] init reaped observer status=0\b")[0]
    if not markers["parked"] < observer_exit < observer_wait or observer_wait <= markers["disposed"]:
        raise ValueError(f"{path}: independent observer did not complete")
    if before_acquisition and (
        observer_exit >= checkpoint or any("[VFS-R1] B held" in line for line in lines)
    ):
        raise ValueError(f"{path}: invalid before-acquisition checkpoint")
    forbidden = rf"\[VFS-L1C-2\] (binding|committed|enqueue|first dispatch selected) child={child}\b"
    if any(re.search(forbidden, line) for line in lines):
        raise ValueError(f"{path}: private child was published")
    panic = r"KERNEL PANIC|panicked at"
    if any(re.search(panic, line, re.IGNORECASE) for line in lines):
        raise ValueError(f"{path}: panic in serial")
    return dict(
        serial=path.name, service=service, child=child, epoch=epoch, retain=retain,
        incarnation=incarnation, vm_generation=vm, service_frames=service_frames,
        service_page_tables=service_tables, service_swap_blocks=service_swap,
        observer=observer, descriptors=descriptors, frames=frames, page_tables=tables,
        swap_blocks=swap, lines=dict(markers, observer_exit=observer_exit, observer_wait=observer_wait),
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("serial", type=Path, nargs="+")
    parser.add_argument("--before-acquisition", action="store_true")
    args = parser.parse_args()
    print(json.dumps([check(path, args.before_acquisition) for path in args.serial], indent=2))
