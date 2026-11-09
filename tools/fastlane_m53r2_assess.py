#!/usr/bin/env python3
"""Fail closed on the wider boot IPC diagnostics; this is not a race assessor."""
import re


SUCCESS = (
    '[SEC]  IPC deferred reply lifecycle: OK',
    '[SEC]  IPC deadline wake/race/late reply: OK',
    '[SEC]  IPC SMP terminal-state stress: 64 iterations OK',
)
ASSERTIONS = (
    'recv_woke', 'recv_timeout', 'call_woke', 'call_timeout', 'reply_commit',
    'cancel_after_reply', 'reply_survived', 'deadline_won', 'late_dropped',
    'stale_ignored', 'stress', 'deferred',
)


def assess_ipc(raw, require_details=True):
    errors = []
    lines = raw.splitlines()
    unexpected = [{'line': i + 1, 'text': line} for i, line in enumerate(lines)
                  if 'UNEXPECTED' in line]
    if unexpected:
        errors.append('unexpected guest diagnostic')
    for marker in SUCCESS:
        if lines.count(marker) != 1:
            errors.append('missing or duplicate success: ' + marker)
    if any(m in raw for m in ('PANIC', 'panicked at')):
        errors.append('guest panic')
    detail = {}
    try:
        for line in lines:
            if line.startswith('[IPC-R2] '):
                pairs = re.findall(r'(\w+)=(\S+)', line)
                row = dict(pairs)
                event = row.get('event')
                if len(row) != len(pairs) or event in detail:
                    raise ValueError('duplicate IPC detail')
                if event not in ('identity', 'assertions'):
                    raise ValueError('unknown IPC detail')
                detail[event] = row
        if require_details or detail:
            if set(detail) != {'identity', 'assertions'}:
                raise ValueError('missing IPC detail')
            identity = detail['identity']
            for key in ('pid', 'inc', 'vm', 'call_inc', 'server', 'server_inc', 'server_vm'):
                if int(identity[key]) <= 0:
                    raise ValueError('invalid IPC identity')
            if identity['inc'] != identity['call_inc']:
                raise ValueError('call incarnation differs from live caller')
            if any(identity[a] == identity[b] for a, b in
                   (('pid', 'server'), ('inc', 'server_inc'), ('vm', 'server_vm'))):
                raise ValueError('fixture actors alias')
            for key, value in {'session': 'none', 'channel': 'none', 'mappings': '0',
                               'authority': 'kernel-fixture', 'scope': 'boot-method'}.items():
                if identity[key] != value:
                    raise ValueError('incorrect evidence scope')
            if set(detail['assertions']) != {'event', *ASSERTIONS}:
                raise ValueError('missing or extra assertion')
            if any(detail['assertions'][key] != '1' for key in ASSERTIONS):
                raise ValueError('failed boot IPC assertion')
    except (KeyError, ValueError) as error:
        errors.append(str(error))
    return {'status': 'FAIL' if errors else 'PASS', 'scope': 'boot-method',
            'errors': errors, 'unexpected': unexpected, 'detail': detail,
            'scheduled_lifecycle': 'NOT_RUN', 'authenticated_membership': 'NOT_RUN',
            'quota_receipt_security': 'NOT_RUN'}
