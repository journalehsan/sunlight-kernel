#!/usr/bin/env python3
"""Actual syscall overlap plus the established per-object retirement assessor."""
import re
from fastlane_account_cancel_validate import assess_cycles as assess_lifecycle


def assess_cycles(raw, cpus, fixtures, *, mode, identity=None, failure=None):
    result = assess_lifecycle(raw, cpus, fixtures, mode=mode, identity=identity, failure=failure)
    errors = result['errors']
    def require(value, why):
        if not value: errors.append(why)
    require(mode in (8, 9, 10), 'invalid concurrent mode')
    for cycle in result['cycles']:
        g = cycle['grant']
        object_id = g.get('object')
        rows = []
        for line in raw.splitlines():
            if not line.startswith(('[RC-RACE]', '[RC-OUTCOME]')): continue
            pairs = re.findall(r'(\w+)=(\S+)', line)
            r = dict(pairs)
            require(len(pairs) == len(r), 'duplicate race field')
            if r.get('object') != object_id: continue
            r['_kind'] = line.split()[0]
            rows.append(r)
        def one(kind, event=None, actor=None):
            found = [r for r in rows if r['_kind'] == '['+kind+']'
                     and (event is None or r.get('event') == event)
                     and (actor is None or r.get('actor') == str(actor))]
            require(len(found) == 1, f'exactly one {kind}/{event}/{actor} required')
            return found[0] if len(found) == 1 else {}
        def n(r, k):
            try: return int(r[k])
            except (KeyError, TypeError, ValueError):
                errors.append('invalid race '+k); return -1
        def before(a, b): return n(a, 'seq') < n(b, 'seq')
        armed = one('RC-RACE', 'armed', 0)
        entries = [one('RC-RACE', 'entered', a) for a in (1, 2)]
        returned = [one('RC-RACE', 'returned', a) for a in (1, 2)]
        held = one('RC-RACE', 'lock-held')
        contention = one('RC-RACE', 'lock-contended')
        winner = n(held, 'winner')
        require(winner in (1, 2) and held.get('actor') == held.get('winner'), 'invalid winner')
        require(mode == 10 or winner == (1 if mode == 8 else 2), 'wrong forced winner')
        require(n(contention, 'actor') == 3-winner and contention.get('winner') == held.get('winner'), 'loser did not observe actual terminal lock contention')
        require(len(rows) == 9, 'missing/extra race events')
        require(n(entries[0], 'cpu') != n(entries[1], 'cpu'), 'same-CPU sequential invocation')
        for r in rows:
            require(r.get('caller') == g.get('caller') and r.get('caller_inc', r.get('inc')) == g.get('caller_inc')
                    and all(r.get(k) == g.get(k) for k in ('endpoint', 'generation')), 'race binding mismatch')
            require(0 <= n(r, 'cpu') < cpus, 'invalid race CPU')
            if r['_kind'] == '[RC-RACE]':
                require(r.get('mode') == str(mode), 'race mode mismatch')
                issuer = r.get('actor') in ('0', '1')
                require(r.get('pid') == g.get('issuer' if issuer else 'caller')
                        and r.get('inc') == g.get('issuer_inc' if issuer else 'caller_inc'), 'actor authority identity mismatch')
        for entry in entries:
            require(before(armed, entry) and before(entry, held), 'actual invocations did not overlap before terminal lock')
        terminal = cycle['terminal'][0] if len(cycle['terminal']) == 1 else {}
        require(before(held, contention) and before(contention, terminal), 'terminal committed before independent contention witness')
        for actor, ret in enumerate(returned, 1):
            require(before(terminal, ret), 'syscall returned before terminal')
            require(ret.get('cpu') == entries[actor-1].get('cpu'), 'actor moved CPUs inside syscall')
        if winner in (1, 2):
            require(before(returned[winner-1], returned[2-winner]), 'loser returned before winner committed')
        # Cancellation is idempotent by ABI: losing cancellation returns zero
        # but must preserve the committed reply and its exact generation.
        require(returned[1].get('result') == '0', 'cancellation syscall failed')
        require(returned[0].get('result') == ('0' if winner == 1 else '4'), 'completion losing result incorrect')
        expected = ('ReplyDelivered' if winner == 1 else 'ExplicitlyCancelled')+'('+g.get('generation', '')+')'
        require(terminal.get('outcome') == expected, 'selected winner differs from actual terminal')
        for actor in (1, 2):
            outcome = one('RC-OUTCOME', actor=actor)
            require(outcome.get('outcome') == 'Some('+expected+')' and outcome.get('pending') == '0'
                    and outcome.get('reply') == ('1' if winner == 1 else '0'), 'independent post-operation outcome mismatch')
            require(outcome.get('inc') == g.get('caller_inc') and before(returned[actor-1], outcome), 'post-operation caller identity/order mismatch')
        states = cycle['records']
        late = [[r for r in states if r['_kind'] == '[CE-STATE]' and r.get('event') == event]
                for event in ('race-late-cancel-before', 'race-late-cancel-after')]
        require(all(len(s) == 1 for s in late), 'late cancellation snapshots missing')
        if all(len(s) == 1 for s in late):
            a, b = late[0][0], late[1][0]
            require(before(a, b) and all(a.get(k) == b.get(k) for k in ('pid','inc','vm','pending','outcome','reply','grants','receipts','deliveries'))
                    and a.get('outcome') == 'Some('+expected+')', 'late cancellation changed terminal or grant inventory')
        users = [dict(re.findall(r'(\w+)=(\S+)', l)) for l in raw.splitlines() if l.startswith('[CE-USER]')]
        supervisors = [r for r in users if r.get('event') == 'foreign-live-authority-denied' and r.get('child') == g.get('caller')]
        require(len(supervisors) == 1 and len([r for r in users if r.get('event') == 'caller-result-matched'
                and r.get('child') == g.get('caller') and r.get('mode') == str(mode)
                and r.get('pid') == supervisors[0].get('pid')]) == 1,
                'independent original result/grant acknowledgement missing')
        require(len([r for r in users if r.get('event') == 'late-completions-rejected'
                     and r.get('child') == g.get('caller') and r.get('pid') == g.get('issuer')]) == 1,
                'late/duplicate completion rejection missing')
        cycle['concurrent_race'] = dict(winner='completion' if winner == 1 else 'cancellation',
            overlap='OBSERVED' if not errors else 'FAILED', events=rows,
            losing_cancellation='IDEMPOTENT_REPLY_PRESERVED' if winner == 1 else None)
        ordered = [dict(re.findall(r'(\w+)=(\S+)', l)) for l in raw.splitlines()
                   if l.startswith(('[AE-', '[CE-', '[RC-'))
                   and dict(re.findall(r'(\w+)=(\S+)', l)).get('object', dict(re.findall(r'(\w+)=(\S+)', l)).get('grant')) == object_id
                   and 'seq=' in l]
        seqs = [n(r, 'seq') for r in ordered]
        require(seqs == sorted(set(seqs)) and all(s > 0 for s in seqs), 'duplicate/reordered combined lifecycle and race trace')
    valid = not errors
    result.update(status='PASS' if valid else 'FAIL', classification='FORCED_AND_PASSED' if valid else 'FORCED_AND_FAILED',
                  terminal_protocol='GUEST_VALIDATED_PENDING_HOST_ACK' if valid else 'BLOCKED',
                  concurrent_invocation_overlap='OBSERVED' if valid else 'FAILED',
                  milestone='SCOPED_RACE_PASS' if valid else 'PARTIAL')
    return result
