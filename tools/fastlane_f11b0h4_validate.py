#!/usr/bin/env python3
"""H4 acceptance: authenticated boundaries plus independent terminal receipts.

The host acknowledgement is written by the native runner only after this
assessment succeeds with current source/image identity. Trace sequence orders
observations; production mutations under SCHEDULER establish linearization.
"""
import json
from pathlib import Path
import re
import sys

from fastlane_f11b0h3_trace import assess as trace_assess


def host_ack_errors(ack, *, fixture, case, cpus, serial_sha256, iso_sha256, elf_sha256):
    """A parsed guest success cannot supply this independent host receipt."""
    if not isinstance(ack, dict):
        return ['host terminal acknowledgement missing']
    expected = {'fixture': fixture, 'case': case, 'cpus': cpus,
                'serial_sha256': serial_sha256, 'iso_sha256': iso_sha256,
                'elf_sha256': elf_sha256, 'security_assertions': 'PASS', 'cleanup': 'PASS'}
    errors = ['host acknowledgement mismatch: ' + key for key, value in expected.items()
              if ack.get(key) != value]
    if not all(ack.get('identity', {}).get(k) for k in ('iso_matches', 'elf_matches', 'services_match', 'source_matches')):
        errors.append('host acknowledgement identity not validated')
    return errors


def rows(raw, prefix):
    return [dict(re.findall(r'(\w+)=([^\s]+)', line))
            for line in raw.splitlines() if line.startswith(prefix)]


def reaper_userspace_errors(raw, fixture):
    """Additional N6 assertions; the full H4 assessment must pass separately."""
    errors = []
    def require(condition, message):
        if not condition:
            errors.append(message)
    try:
        records = rows(raw, '[H3-TRACE] ')
        e = lambda n: [r for r in records if r.get('event') == str(n)]
        arms = e(1)
        if len(arms) != 1:
            return ['userspace reaper authenticated arm missing']
        arm = arms[0]
        issuer = arm['pid']
        require(arm['a'] == '6', 'userspace reaper requires N6')
        ready = rows(raw, '[REAP-TEST] ')
        require(len(ready) == 1 and ready[0].get('fixture') == fixture
                and ready[0].get('checkpoint') == 'issuer-userspace-ready'
                and ready[0].get('issuer') == issuer,
                'correlated ring-3 readiness missing')
        observations = rows(raw, '[SESSION-E3] ')
        commits = [r for r in observations if r.get('checkpoint') == 'mint-committed'
                   and r.get('pid') == issuer and r.get('incarnation') == arm['inc']]
        require(len(commits) == 1 and commits[0].get('result') == '0',
                'genuine committed old grant missing')
        outcomes = [r for r in e(4) + e(5) if r.get('pid') == issuer]
        require(len(outcomes) == 1 and outcomes[0]['event'] == '4'
                and outcomes[0]['inc'] == arm['inc']
                and int(outcomes[0]['seq']) < int(arm['seq']),
                'expected one old grant commit before postcommit rendezvous and no later outcome')
        require(bool(re.search(r'process_mark_finished pid=' + re.escape(issuer)
                + r" name='uac_service' reason=pending-fatal-signal code=143", raw)),
                'ordinary timer fatal-SIGTERM path missing')
        paths = [r for r in e(10) if r['pid'] == issuer and r['inc'] == arm['inc']]
        require(bool(paths) and all(r['a'] == '2' for r in paths),
                'old endpoint was not closed through resource reap')
        retirement = [r for r in e(20) if r['a'] == arm['b'] and r['b'] == arm['c']
                      and r['c'] == issuer]
        closes = e(30)
        removed = [r for r in e(11) if r['pid'] == issuer and r['b'] == '2' and r['c'] == '1']
        require(len(retirement) == len(closes) == len(removed) == 1,
                'single pending-call reap/retirement evidence missing')
        if len(retirement) == len(closes) == len(removed) == 1 and paths:
            path, close, retire, removal = paths[0], closes[0], retirement[0], removed[0]
            require(int(path['seq']) < int(removal['seq']) < int(close['seq']) < int(retire['seq'])
                    and path['cpu'] == removal['cpu'] == close['cpu'] == retire['cpu']
                    and removal['a'] == close['a'],
                    'matching same-CPU peer-close-first reap order missing')
        inventory = [r for r in observations if r.get('checkpoint') == 'inventory']
        require(any(all(r.get(k) == '1' for k in ('receipts', 'account_grants', 'pending_account_deliveries'))
                    for r in inventory), 'committed grant was not pending delivery')
        require(bool(inventory) and inventory[-1].get('observed_mints') == '1'
                and inventory[-1].get('observed_consumes') == '0',
                'unexpected grant mint/consume count')
        clients = rows(raw, '[SESSION-E3-CLIENT] ')
        cleaned = [r for r in clients if r.get('checkpoint') == 'userspace-readiness-cleaned'
                   and r.get('fixture') == fixture and r.get('case') == 'N6' and r.get('result') == '0']
        require(len(cleaned) == 1, 'userspace readiness mapping cleanup missing')
    except (KeyError, ValueError, IndexError, TypeError):
        errors.append('malformed userspace reaper evidence')
    return errors


def guest_terminal_errors(raw, case, fixture):
    errors = []
    for line in raw.splitlines():
        if line.startswith(('[SESSION-E3', '[H3-TRACE]', '[H4-')):
            pairs = re.findall(r'(\w+)=([^\s]+)', line)
            if len(dict(pairs)) != len(pairs):
                errors.append('duplicate marker field')
    clients = rows(raw, '[SESSION-E3-CLIENT] ')
    selected = [r for r in clients if r.get('case') == case and r.get('fixture') == fixture]
    def event(name):
        return [r for r in selected if r.get('checkpoint') == name]
    if any(r.get('fixture') != fixture or r.get('case') != case for r in clients):
        errors.append('stale marker or incorrect fixture ID')
    if 'checkpoint=FAIL' in raw or 'PANIC' in raw or 'panicked at' in raw:
        errors.append('guest assertion failed')
    for name in ('request-start', 'controller-ready', 'policy-accepted-rendezvous',
                 'session-owner-result' if case.startswith('S') else 'old-call-peer-closed', 'manager-restart-response',
                 'post-session-owner-denied' if case.startswith('S') else 'replacement-old-session-denied', 'fixture-complete'):
        if len(event(name)) != 1:
            errors.append('missing or duplicate ' + name)
    if event('replacement-old-session-denied') and event('replacement-old-session-denied')[0].get('result') != '11':
        errors.append('replacement denial is not SessionAuthorityUnavailable')
    if case.startswith('S') and event('post-session-owner-denied') and event('post-session-owner-denied')[0].get('result') not in ('1', '5'):
        errors.append('post-session-owner request not rejected at existing session policy/transport')
    if event('manager-restart-response') and event('manager-restart-response')[0].get('result') != '0':
        errors.append('controller child failed')
    if not event('fixture-complete'):
        return errors
    complete = event('fixture-complete')[0]
    if complete.get('result') != '0' or complete.get('uid') != '0':
        errors.append('unsuccessful terminal marker')
    ctl = [r for r in rows(raw, '[H4-CTL] ') if r.get('checkpoint') == 'ready' and r.get('fixture') == fixture]
    if len(ctl) != 1:
        errors.append('controller READY missing or duplicated')
    pids = [complete['pid']]
    if event('controller-ready'):
        pids.append(event('controller-ready')[0]['pid'])
    if ctl:
        pids.append(ctl[0]['pid'])
        pids.append(ctl[0]['target'])
    clean = rows(raw, '[SESSION-E3] ')
    for pid in pids:
        if not re.search(r"\[SCHED\] process_reaped pid=" + re.escape(pid) + r" name='", raw):
            errors.append('process not reaped: ' + pid)
    for pid in pids[:3]:
        receipts = [r for r in clean if r.get('checkpoint') == 'fixture-ipc-cleaned' and r.get('pid') == pid]
        if len(receipts) != 1 or any(receipts[0].get(k) != v for k, v in
                {'owned_endpoints': '0', 'pending_call': 'none', 'reply_target': 'none',
                 'deferred_targets': '0', 'pending_commits': '0'}.items()):
            errors.append('terminal IPC cleanup missing: ' + pid)
    if not re.search(r"process_mark_finished pid=" + re.escape(complete['pid']) + r" name='session-race-probe' reason=exit code=0", raw):
        errors.append('fixture did not exit zero')
    trace = trace_assess(raw)
    if trace['classification'] != 'OBSERVATIONS_ONLY':
        errors.extend(trace['errors'])
    if not any(r.get('event') == '80' and r.get('a') == fixture and r.get('pid') == complete['pid']
               for r in rows(raw, '[H3-TRACE] ')):
        errors.append('correlated security terminal trace missing')
    return errors


def assess(raw, case, cpus, fixture, *, identity=None, failure=None):
    try:
        return _assess(raw, case, cpus, fixture, identity=identity, failure=failure)
    except (ValueError, KeyError, TypeError, IndexError) as error:
        return {'classification': 'FORCED_AND_FAILED', 'terminal_protocol': 'BLOCKED',
                'host_terminal_acknowledged': False, 'errors': ['malformed trace: ' + type(error).__name__]}


def _assess(raw, case, cpus, fixture, *, identity=None, failure=None):
    errors = guest_terminal_errors(raw, case, fixture)
    records = [r for r in rows(raw, '[H3-TRACE] ') if 'seq' in r]
    valid_events = {1, 2, 3, 4, 5, 6, 10, 11, 20, 30, 40, 41, 42, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 73, 74, 80, 90}
    if any(int(r.get('event', '0')) not in valid_events for r in records):
        errors.append('unknown trace event')
    observations = rows(raw, '[SESSION-E3] ')
    clients = rows(raw, '[SESSION-E3-CLIENT] ')
    def e(number):
        return [r for r in records if r.get('event') == str(number)]
    def require(condition, message):
        if not condition:
            errors.append(message)
    def before(a, b):
        return bool(a and b and int(a[0]['seq']) < int(b[0]['seq']))
    arms = e(1)
    ordinal = {'N4': 4, 'N6': 6, 'H4R': 12, 'S1': 13, 'S2': 14, 'S3': 15}.get(case)
    require(len(arms) == 1 and arms[0].get('a') == str(ordinal), 'wrong authenticated fixture case')
    ready, submitted, accepted = e(60), e(61), e(62)
    require(len(ready) == len(submitted) == len(accepted) == 1, 'controller/manager checkpoints incomplete')
    require(before(ready, arms) and before(arms, submitted) and before(submitted, accepted), 'prepare/release order missing')
    require(bool(ready) and ready[0]['a'] == fixture and ready[0]['pid'] == submitted[0]['pid'] if submitted else False,
            'controller correlation mismatch')
    require(bool(ready and accepted) and ready[0]['b'] == accepted[0]['b'] and ready[0]['pid'] == accepted[0]['c'],
            'manager accepted a different caller/target')
    activations = [r for r in observations if r.get('checkpoint') == 'account-activated']
    registrations = [r for r in observations if r.get('checkpoint') == 'registered']
    login_proofs = [r for r in observations if r.get('checkpoint') == 'login-proof-committed']
    if activations and login_proofs:
        require(login_proofs[0].get('account') == '1' and all(login_proofs[0].get(k) == activations[0].get(k)
                for k in ('pid', 'incarnation', 'vm')), 'login proof lacks genuine Account provenance')
    owner_case = case.startswith('S')
    require(len(activations) == (1 if owner_case else 2) and len(registrations) == 1 and bool(login_proofs), 'authenticated activation/login/registration missing')
    if len(activations) == 2 and arms:
        old, new = activations
        require(all(arms[0].get(a) == old.get(b) for a, b in (('pid', 'pid'), ('inc', 'incarnation'))), 'issuer not activated Account')
        require(old['incarnation'] != new['incarnation'] and old['vm'] != new['vm'], 'replacement identity not distinct')
        require(bool(ready) and ready[0]['b'] == old['pid'], 'controller resolved wrong incarnation')
    if owner_case and activations and arms and registrations and ready:
        require(arms[0]['pid'] == activations[0]['pid'] and arms[0]['inc'] == activations[0]['incarnation'], 'wrong Account issuer')
        require(ready[0]['b'] == registrations[0]['pid'], 'controller did not resolve session owner')
    if registrations and arms:
        require(arms[0]['b'] == registrations[0]['session'] and arms[0]['c'] == registrations[0]['epoch']
                and registrations[0].get('sessiond') == '1', 'session authority mismatch')
    commits = [r for r in e(4) if arms and r['pid'] == arms[0]['pid'] and r['inc'] == arms[0]['inc']]
    rejects = [r for r in e(5) if arms and r['pid'] == arms[0]['pid'] and r['inc'] == arms[0]['inc']]
    retirement = [r for r in e(20) if arms and r['a'] == arms[0]['b'] and r['b'] == arms[0]['c']]
    require(len(retirement) == 1 and bool(e(10)) and bool(e(11)) and (owner_case or bool(e(30))), 'retirement/path/peer close evidence incomplete')
    # Exact pending recipient incarnation and IPC call generation are separate
    # from the endpoint ID. Correlate the authoritative call cancellation.
    requesters = [r for r in clients if r.get('checkpoint') == 'request-start']
    bindings = [r for r in observations if r.get('checkpoint') == 'request-binding']
    closes = [r for r in e(30) if requesters and r['pid'] == requesters[0]['pid']]
    if not owner_case:
        require(len(closes) == 1 and any(b.get('recipient') == c['pid'] and b.get('recipient_inc') == c['inc']
                and b.get('request_generation') == c['b'] for b in bindings for c in closes), 'pending call identity/cancellation missing')
    inventories = [r for r in observations if r.get('checkpoint') == 'inventory']
    require(bool(inventories) and all(inventories[-1].get(k) == '0' for k in
            ('sessions', 'receipts', 'account_grants', 'pending_account_deliveries')), 'fixture broker inventory not empty')
    require(not any(int(r.get('cpu', '0')) >= cpus for r in records), 'invalid CPU identity')
    require(len({r['cpu'] for r in records}) >= 2, 'no SMP participation')
    if case == 'N4':
        require(len(commits) == 1 and not rejects and before(commits, retirement), 'grant-wins commitment not proven')
        require(before(ready, e(65)) and before(e(65), accepted), 'controller not ready before grant checkpoint')
        require(before(e(64), commits) and before(commits, e(70)) and before(e(70), e(71)), 'signal/grant/exit sequence incomplete')
        require(bool(e(65) and e(64)) and e(65)[0]['cpu'] != e(64)[0]['cpu'], 'manager and grant did not execute on distinct CPUs')
    elif case == 'H4R':
        require(not commits and len(rejects) == 1, 'finished issuer committed or did not reject')
        require(before(e(67), e(66)) and before(e(66), rejects) and before(rejects, retirement), 'forced finish/rejection/retirement sequence incomplete')
        require(bool(e(67)) and e(67)[0]['b'] == '1' and e(67)[0]['a'] != '255', 'execution ownership prevention missing')
        require(any(r['b'] == '9' for r in e(64)), 'ordinary manager SIGKILL escalation missing')
    elif owner_case:
        require(before(ready, e(65)) and before(e(65), accepted) and before(accepted, e(70)), 'owner contender preparation order missing')
        require(before(e(68), e(3)) and before(e(70), e(3)) and before(e(68), e(71)) and before(e(70), e(71)), 'both competitors were not ready before either lock acquisition')
        require(bool(e(68) and e(70)) and e(68)[0]['cpu'] != e(70)[0]['cpu'], 'contenders did not run on distinct CPUs')
        require(bool(retirement and ready) and retirement[0]['c'] == ready[0]['b'], 'wrong retiring session owner')
        if case == 'S1':
            require(len(commits) == 1 and before(commits, e(71)) and before(commits, retirement), 'grant-first order missing')
        elif case == 'S2':
            require(not commits and before(retirement, rejects), 'retirement-first rejection missing')
            require(any(r.get('checkpoint') == 'mint-syscall-rejected' and arms
                        and r.get('pid') == arms[0]['pid'] and r.get('incarnation') == arms[0]['inc']
                        and r.get('result') == '11' for r in observations), 'broker session authority rejection missing')
        require(not e(90) and 'checkpoint=boundary-timeout' not in raw, 'rendezvous timed out')
        require(not any(before(retirement, [commit]) for commit in commits), 'unauthorized post-retirement commitment')
    elif case != 'N6':
        errors.append('unsupported H4 case')
    if not identity or not all(identity.get(k) for k in ('iso_matches', 'elf_matches', 'services_match', 'source_matches')):
        errors.append('missing or mismatched source/image identity')
    if failure:
        errors.append('driver failure: ' + failure['type'])
    valid = not errors
    blocked_grant = [r for r in e(69) if before(e(2), [r]) and before([r], e(3))
                     and before(e(71), [r]) and before([r], e(74))]
    blocked_reaper = [r for r in e(73) if ready and r['pid'] == ready[0]['b']
                      and before(e(3), [r]) and before([r], e(6)) and before([r], e(71))]
    if case == 'S3' and not (blocked_grant or blocked_reaper):
        errors.append('no failed acquisition inside the other contender held-lock interval')
        valid = False
    contention = owner_case and bool(blocked_grant or blocked_reaper) and valid
    return {'classification': ('FORCED_AND_PASSED' if owner_case else 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT') if valid else 'FORCED_AND_FAILED',
            'case': case, 'fixture': fixture, 'errors': errors, 'identity': identity,
            'terminal_protocol': 'GUEST_VALIDATED_PENDING_HOST_ACK' if valid else 'BLOCKED', 'host_terminal_acknowledged': False,
            'controller_ready': bool(ready), 'manager_accepted': bool(accepted),
            'reaper_contender_observed': valid and owner_case, 'both_contenders_ready': valid and owner_case,
            'genuine_reaper_contention': 'FORCED_AND_PASSED' if contention else 'NOT_FORCED',
            'failed_acquisitions_while_peer_held_lock': blocked_grant + blocked_reaper,
            'grant_wins': 'FORCED_AND_PASSED' if valid and case in ('N4', 'S1') else 'NOT_FORCED',
            'retirement_wins': ('FORCED_AND_PASSED' if owner_case else 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT') if valid and case in ('H4R', 'S2') else 'NOT_FORCED',
            'path_events': e(10), 'retirement': retirement, 'peer_closed': closes,
            'peer_close_first': 'OBSERVED' if before(closes, retirement) else 'NOT_FORCED',
            'grant_commits': commits, 'grant_rejects': rejects,
            'cpu_participants': sorted({int(r['cpu']) for r in records}),
            'controller_and_manager': ready + submitted + accepted,
            'lock_observations': [r for r in records if r['event'] in ('2', '3', '6', '63', '64', '65', '66', '67', '68', '69', '70', '71', '73', '74', '90')],
            'inventory': inventories[-1:]}


if __name__ == '__main__':
    import fastlane_f11b0e3_validate as runner
    sys.argv.extend(['--h4'])
    if '--evidence-root' not in sys.argv:
        sys.argv.extend(['--evidence-root', str(runner.ROOT / 'target/fastlane-f11b0h4-evidence')])
    runner.main()
