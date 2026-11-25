#!/usr/bin/env python3
"""Check transaction ordering in a native delayed completion serial log."""

import argparse
import json
import re
from pathlib import Path


def check(path, teardown):
    lines = path.read_text(errors="replace").splitlines()
    transactions = []

    def only(pattern):
        matches = [index + 1 for index, line in enumerate(lines) if re.search(pattern, line)]
        if len(matches) != 1:
            raise ValueError(f"{path}: expected one {pattern!r}; found {matches}")
        return matches[0]

    starts = [index + 1 for index, line in enumerate(lines)
              if "[VFS-R4] A before construction adapter=empty" in line]
    if len(starts) != 8:
        raise ValueError(f"{path}: expected eight construction checkpoints")
    services = set()
    for line in lines:
        match = re.search(r"\[VFS-R4\] B held service=(\d+) child=(\d+) retain=(\d+) epoch=(\d+)", line)
        if not match:
            continue
        service, child, retain, epoch = map(int, match.groups())
        services.add((service, epoch))
        positions = {
            "before": starts[len(transactions)],
            "adapter": only(rf"acquire child={child} retain={retain} epoch={epoch} mode=4\b"),
            "held": only(rf"B held service={service} child={child} retain={retain} epoch={epoch}\b"),
        }
        if teardown:
            positions["vm_reclaimed"] = only(rf"teardown child={child} descriptors=3 frames=[1-9]\d* tables=[1-9]\d* swap=0\b")
            positions["heap_dropped"] = only(rf"dropped child={child} process_box=1 fd_table_box=1\b")
        positions.update(
            cancelled=only(rf"cancelled mode=4 child={child} other=[1-9]\d* reclaimed=1\b"),
            disposed=only(rf"dispose child={child} retain={retain} owner=reclaimed\b"),
            reclaimed=only(rf"C reclaimed child={child} retain={retain} slot=\d+ pid_reserved=1 continuation=0\b"),
            late=only(rf"D late child={child} retain={retain} slot=\d+ compensation=1 continuation=0\b"),
            settled=only(rf"settled child={child} retain={retain} epoch={epoch}\b"),
        )
        order = list(positions.values())
        if any(left >= right for left, right in zip(order, order[1:])):
            raise ValueError(f"{path}: child {child} out of order: {positions}")
        if any(re.search(rf"\[VFS-L1C-2\] (binding|committed|enqueue|first dispatch selected) child={child}\b", row)
               for row in lines):
            raise ValueError(f"{path}: cancelled child {child} was published")
        transactions.append(dict(child=child, retain=retain, service=service, epoch=epoch, lines=positions))
    if len(services) != 1 or len(transactions) != len(starts):
        raise ValueError(f"{path}: expected eight transactions and one service registration")
    only(r"\[VFS-R4\] init completed cycles=8\b")
    return dict(serial=str(path), transactions=transactions)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--initial", action="store_true", help="check the first build without teardown counters")
    parser.add_argument("serial", type=Path, nargs="+")
    options = parser.parse_args()
    print(json.dumps([check(path, not options.initial) for path in options.serial], indent=2))
