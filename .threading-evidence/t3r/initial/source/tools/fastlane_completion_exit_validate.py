#!/usr/bin/env python3
"""Actual userspace exit/completion overlap; reuse physical-object verification."""
import re
from fastlane_account_cancel_validate import assess_cycles as assess_lifecycle


def assess_cycles(raw, cpus, fixtures, *, mode, identity=None, failure=None):
    result = assess_lifecycle(raw, cpus, fixtures, mode=mode, identity=identity, failure=failure)
    errors = result['errors']
    def require(value, why):
        if not value:
            errors.append(why)
    require(mode in (11, 12, 13), 'invalid actual-exit mode')
    all_rows = []
    for line in raw.splitlines():
        if not line.startswith(('[EX-RACE]', '[EX-OUTCOME]')):
            continue
        fields = re.findall(r'(\w+)=(\S+)', line)
        r = dict(fields)
        require(len(fields) == len(r), 'duplicate exit race field')
        r['_kind'] = line.split()[0]
        all_rows.append(r)
    for cycle in result['cycles']:
        g = cycle['grant']
        rows = [r for r in all_rows if r.get('object') == g.get('object')]
        def one(kind, event=None, actor=None):
            found = [r for r in rows if r['_kind'] == '['+kind+']'
                     and (event is None or r.get('event') == event)
                     and (actor is None or r.get('actor') == str(actor))]
            require(len(found) == 1, f'exactly one {kind}/{event}/{actor} required')
            return found[0] if len(found) == 1 else {}
        def n(r, k):
            try:
                return int(r[k])
            except (KeyError, ValueError, TypeError):
                errors.append('invalid exit race '+k)
                return -1
        def before(a, b):
            return n(a, 'seq') < n(b, 'seq')
        armed = one('EX-RACE', 'armed', 0)
        syscall = one('EX-RACE', 'exit-syscall', 2)
        entries = [one('EX-RACE', 'entered', a) for a in (1, 2)]
        held = one('EX-RACE', 'lock-held')
        contention = one('EX-RACE', 'lock-contended')
        returned = one('EX-RACE', 'returned', 1)
        exited = one('EX-RACE', 'exit-committed', 2)
        marked = one('EX-RACE', 'exit-marked', 2)
        outcomes = [one('EX-OUTCOME', actor=a) for a in (1, 2)]
        winner = n(held, 'winner')
        require(winner in (1, 2) and held.get('actor') == str(winner), 'invalid selected exit winner')
        require(mode == 13 or winner == (1 if mode == 11 else 2), 'wrong forced exit winner')
        require(contention.get('actor') == str(3-winner) and contention.get('winner') == str(winner), 'independent loser contention absent')
        require(len(rows) == 11, 'missing/extra actual exit race events')
        require(n(entries[0], 'cpu') != n(entries[1], 'cpu'), 'actual actors ran sequentially on same CPU')
        require(before(armed, syscall) and before(syscall, entries[1]), 'actual exit syscall absent before exit lifecycle entry')
        require(syscall.get('cpu') == entries[1].get('cpu') == exited.get('cpu'), 'exit syscall changed CPU inside kernel')
        for entry in entries:
            require(before(armed, entry) and before(entry, held), 'actual syscalls not both active before contested lock')
        require(before(held, contention) and before(contention, returned) and before(contention, exited), 'lifecycle won before independent contention')
        first, second = (returned, exited) if winner == 1 else (exited, returned)
        require(before(first, second), 'actual lifecycle order differs from selected winner')
        require(returned.get('result') == ('0' if winner == 1 else '4') and exited.get('result') == '0', 'wrong losing completion or exit result')
        require(returned.get('cpu') == entries[0].get('cpu'), 'Account migrated inside syscall')
        for r in rows:
            require(r.get('caller') == g.get('caller') and r.get('caller_inc', r.get('inc')) == g.get('caller_inc')
                    and all(r.get(k) == g.get(k) for k in ('endpoint', 'generation')), 'exit race exact binding mismatch')
            require(0 <= n(r, 'cpu') < cpus, 'invalid actual exit CPU')
            if r['_kind'] == '[EX-RACE]':
                require(r.get('mode') == str(mode), 'actual exit mode mismatch')
                issuer = r.get('actor') in ('0', '1')
                require(r.get('pid') == g.get('issuer' if issuer else 'caller')
                        and r.get('inc') == g.get('issuer_inc' if issuer else 'caller_inc'), 'actual syscall actor identity mismatch')
        for i, outcome in enumerate(outcomes):
            require(before((returned, exited)[i], outcome), 'post-transition snapshot precedes operation')
            require(outcome.get('inc') == g.get('caller_inc'), 'snapshot adopted stale incarnation')
        require(outcomes[1].get('state') == 'Finished' and outcomes[1].get('maps') == '1', 'original caller did not actually exit with owned mapping')
        if winner == 1:
            terminal = cycle['terminal'][0] if len(cycle['terminal']) == 1 else {}
            require(before(contention, terminal) and before(terminal, returned), 'completion publication outside contested transition')
            for outcome in outcomes:
                require(outcome.get('pending') == '0' and outcome.get('reply') == '1'
                        and outcome.get('outcome') == 'Some(ReplyDelivered('+g.get('generation', '')+'))', 'exit replaced committed completion')
            require(outcomes[0].get('grants') == outcomes[0].get('receipts') == '1'
                    and outcomes[0].get('deliveries') == '0', 'valid completed grant retired prematurely')
            require(before(returned, marked) and before(marked, cycle['retirement']) and before(cycle['retirement'], exited), 'caller grant retirement outside actual exit transition')
        else:
            require(not cycle['terminal'], 'reply published to retired caller')
            require(all(o.get('reply') == '0' and o.get('outcome') == 'None' for o in outcomes), 'exit loser created reply or second terminal')
            require(outcomes[0].get('state') in ('Finished', 'Reaped'), 'completion did not observe exiting original caller')
        require(before(contention, marked) and before(marked, exited), 'exit state publication outside contested transition')
        reaped = next((r for r in cycle['records'] if r['_kind'] == '[CE-REAP]' and r.get('event') == 'reap-clean'), {})
        require(before(exited, reaped), 'reaped before actual syscall exit')
        require(before(marked, cycle['retirement']) and before(cycle['retirement'], exited), 'grant retirement outside actual exit cleanup')
        require(all(outcomes[1].get(k) == '0' for k in ('grants', 'receipts', 'deliveries')), 'actual exit left grant or receipt inventory')
        require(re.search(r'\[SCHED\] process_mark_finished pid='+re.escape(g.get('caller', '?'))+r" name='session-racer' reason=exit code=0\b", raw) is not None, 'actual caller exit status absent')
        users = [dict(re.findall(r'(\w+)=(\S+)', l)) for l in raw.splitlines() if l.startswith('[CE-USER]')]
        for event, pid, child in [('calling-real-exit', g.get('caller'), '0'), ('late-completions-rejected', g.get('issuer'), g.get('caller'))]:
            require(len([r for r in users if r.get('event') == event and r.get('pid') == pid and r.get('child') == child and r.get('mode') == str(mode)]) == 1, 'actual userspace '+event+' missing/duplicated')
        require(not any(r.get('pid') == g.get('caller') and r.get('event') == 'result-consumed' for r in users), 'exited caller resumed and collected a result')
        ordered = []
        for line in raw.splitlines():
            if not line.startswith(('[AE-', '[CE-', '[EX-')):
                continue
            r = dict(re.findall(r'(\w+)=(\S+)', line))
            if r.get('object', r.get('grant')) == g.get('object') and 'seq' in r:
                ordered.append(n(r, 'seq'))
        require(ordered == sorted(set(ordered)) and all(s > 0 for s in ordered), 'duplicated/reordered combined exit trace')
        cycle['concurrent_exit'] = dict(winner='completion' if winner == 1 else 'caller-exit',
            terminal_outcome='ReplyDelivered' if winner == 1 else 'CallerExited',
            overlap='OBSERVED', actual_exit_syscall=True, events=rows)
    require(len(all_rows) == 11*len(fixtures), 'unbound/extra exit race events')
    valid = not errors
    result.update(status='PASS' if valid else 'FAIL', classification='FORCED_AND_PASSED' if valid else 'FORCED_AND_FAILED',
                  terminal_protocol='GUEST_VALIDATED_PENDING_HOST_ACK' if valid else 'BLOCKED',
                  concurrent_invocation_overlap='OBSERVED' if valid else 'FAILED',
                  milestone='SCOPED_EXIT_RACE_PASS' if valid else 'PARTIAL')
    return result
