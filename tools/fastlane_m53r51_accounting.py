#!/usr/bin/env python3
"""Summarize R5.1 primary PMM checkpoints; never infer allocation lifetimes."""
import argparse
import json
from pathlib import Path
import re

CLASSES = (
    'Free', 'ReservedFirmware', 'ReservedKernelImage', 'KernelCore',
    'KernelHeap', 'KernelStack', 'PageTable', 'UserPrivate', 'SharedMemory',
    'RamFsFileData', 'RamFsMetadata', 'FileSystemCache', 'GraphicsBuffer',
    'DeviceDma', 'CompressedMemory', 'OtherAccounted',
)
PMM = re.compile(r'^\[R51-PMM\] label=(\S+) managed=(\d+) free=(\d+) used=(\d+) scope=pmm-lock-snapshot$')
CLASS = re.compile(r'^\[R51-CLASS\] label=(\S+) class=(\d+) frames=(\d+) scope=primary-class-overlaps-owner-and-quota$')


def assess(raw):
    samples = []
    errors = []
    current = None
    for line in raw.splitlines():
        match = PMM.fullmatch(line)
        if match:
            label, managed, free, used = match.groups()
            current = dict(label=label, managed=int(managed), free=int(free),
                           used=int(used), classes={})
            samples.append(current)
            if current['managed'] != current['free'] + current['used']:
                errors.append('PMM conservation mismatch: ' + label)
            continue
        match = CLASS.fullmatch(line)
        if match:
            label, index, frames = match.groups()
            index = int(index)
            if current is None or label != current['label'] or index >= len(CLASSES):
                errors.append('unjoined class row: ' + line)
                continue
            name = CLASSES[index]
            if name in current['classes']:
                errors.append('duplicate class: ' + label + '/' + name)
            current['classes'][name] = int(frames)
    for sample in samples:
        if len(sample['classes']) != len(CLASSES):
            errors.append('incomplete class snapshot: ' + sample['label'])
    before = [s for s in samples if s['label'] == 'baseline']
    after = [s for s in samples if s['label'] == 'quiescent']
    delta = None
    if len(before) != 1 or len(after) != 1:
        errors.append('exactly one baseline and quiescent checkpoint required')
    elif samples.index(before[0]) >= samples.index(after[0]):
        errors.append('quiescent must follow baseline')
    elif not errors:
        a, b = before[0], after[0]
        delta = {k: b[k] - a[k] for k in ('managed', 'free', 'used')}
        delta['classes'] = {k: b['classes'][k] - a['classes'][k] for k in CLASSES}
    return {
        'status': 'FAIL' if errors else 'OBSERVED',
        'attribution': 'UNATTRIBUTED',
        'global_zero_drift_qualified': False,
        'scope': 'PMM class deltas only; no allocation lifetime or quiescence proof',
        'errors': errors, 'samples': samples, 'delta': delta,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('serial', type=Path)
    args = parser.parse_args()
    result = assess(args.serial.read_text(errors='replace'))
    print(json.dumps(result, indent=2))
    return int(result['status'] == 'FAIL')


if __name__ == '__main__':
    raise SystemExit(main())
