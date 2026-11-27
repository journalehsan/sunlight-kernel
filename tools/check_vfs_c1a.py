#!/usr/bin/env python3
"""Verify ordinary namespace publication against a completed isolated serial run."""
import argparse
import json
import re
from pathlib import Path


def verify(serial, exit_status):
    assert exit_status == 0
    lines = serial.read_text().splitlines()

    def matches(pattern):
        return [(i, match) for i, line in enumerate(lines, 1)
                if (match := re.search(pattern, line))]

    def one(pattern):
        [record] = matches(pattern)
        return record

    def fields(match):
        return dict((key, int(value)) for key, value in re.findall(r'(\w+)=(\d+)', match[0]))

    def identity(values):
        return (values['pid'], values['incarnation'], values['image'], values['epoch'], values['retain'])

    ready, _ = one(r'\[init\] VFS namespace ready before service spawn')
    baseline_line, baseline = one(r'\[VFS-C1A\] baseline .*')
    complete, _ = one(r'\[VFS-C1A\] ordinary complete native=4 capability=4 refused_publication=2')
    assert ready < baseline_line < complete
    assert not matches(r'KERNEL PANIC|caller fixture failure|\[init\] PANIC|\[VFS\] PANIC')
    baseline = fields(baseline)
    assert baseline['leases'] == 1
    assert all(baseline[key] == 0 for key in ('pending', 'owned_frames', 'service_rows', 'service_refs'))
    settled = matches(r'\[VFS-C1A\] settled .*')
    assert len(settled) == 10 and all(fields(match) == baseline for _, match in settled)
    attempts = matches(r'\[VFS-C1A\] (bound|publication refused) adapter=(native|capability) .*')
    assert len(attempts) == len(settled)
    users = matches(r'\[VFS-C1A\] user .* cwd=/etc')
    assert len(users) == 8
    assert len({fields(match)['namespace'] for _, match in users}) == 1
    assert len({fields(match)['generation'] for _, match in users}) == 8
    observed = {'native': [], 'capability': []}
    for (start, attempt), (end, _) in zip(attempts, settled, strict=True):
        phase, adapter = attempt.groups()
        values = fields(attempt)
        pid, incarnation, image, epoch, retain = identity(values)
        assert baseline_line < start < end < complete
        record = dict(values, publication_line=start, settlement_line=end)
        user = [(line, fields(match)) for line, match in users
                if identity(fields(match)) == identity(values)]
        if phase == 'publication refused':
            assert not user
            release, _ = one(rf'\[VFS-L1C-1\] settled child={pid} retain={retain} epoch={epoch}\b')
            assert start < release < end
            assert not matches(rf'\[NATIVE-GROUP\] terminal pid={pid} incarnation={incarnation} vm={image}\b')
            record.update(outcome='publication refused', release_line=release)
        else:
            [(entered, scope)] = user
            dispatch, _ = one(rf'\[VFS-C1A\] first dispatch pid={pid} incarnation={incarnation} image={image} cpu=\d+')
            service = (rf'\[VFS-D1B\] service pid={pid} incarnation={incarnation} '
                       rf'image={image} epoch={epoch} .*')
            opened, open_record = one(service + r' op=0 token=0 result=\d+ errno=0 handles=1 ack=1')
            token = fields(open_record)['result']
            read, _ = one(service + rf' op=1 token={token} result=8 errno=0 handles=1 ack=1')
            pread, _ = one(service + rf' op=9 token={token} result=22 errno=0 handles=1 ack=1')
            mapped, _ = one(rf'\[VFS-D2A\] committed pid={pid} .*span=4096 bytes=22 offset=0 prot=0x1')
            exited, _ = one(rf'\[NATIVE-GROUP\] terminal pid={pid} incarnation={incarnation} vm={image} code=37 reason=exit')
            reaped, _ = one(rf"\[SCHED\] process_reaped pid={pid} name='vfs-c1a-probe' .* swap_blocks=0")
            owner, _ = one(rf'\[VFS-D1B\] owner retirement pid={pid} incarnation={incarnation} '
                           rf'image={image} epoch={epoch} handles=[01]->0')
            assert start < dispatch < entered < opened < read < pread < mapped < exited < reaped < end
            assert exited < owner < end
            record.update(outcome='published', dispatch_line=dispatch, user_line=entered,
                          namespace=scope['namespace'],
                          namespace_generation=scope['generation'], retained_pair=scope['pair'],
                          open_line=opened, mmap_line=mapped, exit_line=exited,
                          reap_line=reaped, owner_retirement_line=owner)
        observed[adapter].append(record)
    for records in observed.values():
        assert [record['outcome'] for record in records] == ['publication refused'] + ['published'] * 4
    return dict(status='PASS', serial=str(serial), complete_line=complete,
                baseline=baseline, adapters=observed)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('serial', type=Path)
    parser.add_argument('exit_status', type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(args.serial, int(args.exit_status.read_text())), indent=2))
