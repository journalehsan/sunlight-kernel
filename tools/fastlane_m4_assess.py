"""Fail-closed M4 serial assessment. Synthetic parser controls are not native evidence."""
import re


def rows(raw, prefix):
    result = []
    for line in raw.splitlines():
        if line.startswith(prefix):
            pairs = re.findall(r'(\w+)=(\S+)', line)
            r = dict(pairs)
            if len(r) != len(pairs):
                r['_duplicate'] = '1'
            result.append(r)
    return result


def assess(raw, cpus, identity=None, failure=None):
    errors = []
    def need(ok, reason):
        if not ok:
            errors.append(reason)
    need(cpus in (2, 4, 12), 'unsupported topology')
    need(identity is not None and all(identity.get(k) is True for k in ('source', 'elf', 'iso', 'services')), 'source/image identity missing')
    need(failure is None, 'execution failure')
    need(not any(m in raw for m in ('PANIC', 'panicked at', '[FL-BELL-DEADLINE]', 'checkpoint=FAIL')), 'guest failure')
    cases = rows(raw, '[FL-M4] ')
    need(not any('_duplicate' in r for r in cases), 'duplicate field')
    expected = {'start': [0], 'unauthorized-stolen-stale': [0], 'notify-before-wait': [1],
                'sleeping-repeated': [9], 'registration-race': [1, 2],
                'cancel-notify-orders': [0, 1], 'signal-cancel-notify': [5, 6], 'consumer-killed-wait': [0], 'revocation-wait': [0], 'producer-exit-wait': [0],
                'consumer-exit-notify': [3, 4], 'sequence-exhaustion': [0],
                'reclaimed-cycle': list(range(16)), 'exec-authority-retired': [0], 'complete': [0]}
    need(set(r.get('case') for r in cases) == set(expected), 'case set mismatch')
    for case, details in expected.items():
        need(sorted(r.get('detail', '') for r in cases if r.get('case') == case)
             == sorted(str(d) for d in details), 'missing/duplicate case: ' + case)
    bindings = rows(raw, '[FL-BIND] ')
    attached = [r for r in bindings if r.get('op') == '3' and r.get('state') == '3']
    need(len(attached) == 30, 'missing independent attached channels')
    generations = {r.get('generation') for r in attached}
    for r in attached:
        need(r.get('producer') != r.get('consumer') and r.get('producer_vm') != r.get('consumer_vm'), 'shared participant VM')
        need(all(r.get(k, '0') != '0' for k in ('issuer_inc', 'epoch', 'producer_inc', 'consumer_inc', 'producer_vm', 'consumer_vm')), 'missing authenticated binding')
    events = rows(raw, '[FL-BELL] ')
    registered = [r for r in events if r.get('event') == 'registered']
    completions = [r for r in events if r.get('event') == 'complete']
    takes = [r for r in events if r.get('event') == 'take']
    need(len(registered) == 33 and len(completions) == len(takes) == 32, 'registration/completion/collection accounting')
    dead = [r.get('generation') for r in cases if r.get('case') == 'consumer-killed-wait']
    dead_regs = [r for r in registered if r.get('generation') in dead]
    need(len(dead_regs) == 1 and not any(r.get('generation') in dead for r in completions + takes), 'dead consumer registration not withdrawn')
    if dead_regs:
        need(('[SCHED] process_reaped pid=' + dead_regs[0]['consumer'] + ' ') in raw, 'missing actual dead-consumer reap')
    resources = rows(raw, '[FL-BELL-RESOURCE] ')
    need(bool(resources), 'resource census missing')
    for r in resources:
        try:
            need(0 < int(r['rows']) <= int(r['capacity']) <= 8, 'unbounded channel metadata')
        except (KeyError, ValueError):
            errors.append('invalid resource row')
    if resources:
        need(all(resources[-1].get(k) == '0' for k in ('active', 'waiters', 'receipts', 'pending')), 'terminal resources remain')
    for case in [r for r in cases if r.get('case') == 'reclaimed-cycle']:
        need(any(r.get('generation') == case.get('generation') and all(r.get(k) == '0' for k in ('active', 'waiters', 'receipts', 'pending')) for r in resources), 'cycle did not reclaim')
    need(all(r.get('blocked') == '1' for r in completions), 'completion did not wake blocked task')
    remote = 0
    for reg, comp, take in zip([r for r in registered if r.get("generation") not in dead], completions, takes):
        need(all(reg.get(k) == comp.get(k) == take.get(k) for k in ('generation', 'consumer', 'inc', 'vm')), 'completion identity mismatch')
        need(comp.get('result') == take.get('result') and comp.get('sequence') == take.get('sequence'), 'completion changed before collection')
        need(reg.get('generation') in generations, 'unbound notification')
        if reg.get('cpu') != comp.get('cpu'):
            remote += 1
    need(remote > 0, 'no cross-CPU wake evidence')
    parks = rows(raw, '[FL-BELL-PARK] ')
    need(len(parks) == 32, 'missing physical park witnesses')
    for r in parks:
        need(all(r.get(k) == v for k, v in {'parked': '1', 'owner': '255', 'queued': '255', 'pins': '0', 'ready': '0'}.items()), 'waiter still executing/queued/pinned')
    races = rows(raw, '[FL-BELL-RACE] ')
    for mode in range(1, 7):
        rs = [r for r in races if r.get('mode') == str(mode)]
        entrants = [r for r in rs if r.get('event') in ('entered', 'exit-syscall')]
        need(len(entrants) == 2 and len({r.get('cpu') for r in entrants}) == 2, 'race not cross-CPU: ' + str(mode))
        need(len([r for r in rs if r.get('event') == 'lock-contended']) == 1, 'missing actual contention: ' + str(mode))
        returned = {r.get('actor'): r.get('result') for r in rs if r.get('event') == 'returned'}
        expected_returns = {1: {'1': '0', '2': '0'}, 2: {'1': '10', '2': '0'},
                            3: {'1': '0', '2': '4'}, 4: {'1': '0', '2': '0'},
                            5: {'1': '0', '2': '0'}, 6: {'1': '0', '2': '0'}}
        need(returned == expected_returns[mode], 'wrong race outcome: ' + str(mode))
    return {'status': 'FAIL' if errors else 'PASS', 'errors': errors,
            'channels': len(attached), 'registrations': len(registered), 'completions': len(completions),
            'collected': len(takes), 'remote_wakes': remote, 'cases': cases, 'resources': resources, 'physical_parks': len(parks)}
