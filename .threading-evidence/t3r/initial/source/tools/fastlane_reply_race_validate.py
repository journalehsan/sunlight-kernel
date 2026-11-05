"""Fail-closed parser for concurrent kernel-method evidence, not Account peers."""
import re


def assess(raw, cpus):
    errors = []
    lines = [line for line in raw.splitlines() if line.startswith('[IPC-RACE] ')]
    records = []
    for line in lines:
        pairs = re.findall(r'(\w+)=(\S+)', line)
        row = dict(pairs)
        if len(row) != len(pairs):
            errors.append('duplicate field')
        records.append(row)

    def require(condition, why):
        if not condition:
            errors.append(why)

    try:
        all_sequences = []
        endpoints, generations = [], []
        identities = []
        for case in range(1, 5):
            selected = [r for r in records if r.get('case') == str(case)]
            def one(event):
                matches = [r for r in selected if r.get('event') == event]
                require(len(matches) == 1, f'case {case}: require one {event}')
                return matches[0]
            identity = one('identity')
            require(identity.get('scope') == 'kernel-method' and identity.get('account_auth') == 'NOT_RUN',
                    'kernel-method scope promoted')
            keys = ('caller', 'caller_inc', 'caller_vm', 'server', 'server_inc', 'server_vm')
            identities.append(tuple(identity[k] for k in keys))
            require(all(int(identity[k]) > 0 for k in keys), 'invalid participant identity')
            require(identity['caller'] != identity['server'], 'same participant')
            endpoints.append(int(identity['endpoint']))
            generations.append(int(identity['generation']))
            pub, close = one('publish-enter'), one('close-enter')
            pub_done, close_done = one('publish-finished'), one('close-committed')
            require(int(pub['cpu']) == int(pub_done['cpu']) == 0
                    and int(close['cpu']) == int(close_done['cpu']) == 1 < cpus,
                    'separate native CPUs required')
            seq = [int(r['seq']) for r in (pub, close, pub_done, close_done)]
            require(max(seq[:2]) < min(seq[2:]), 'operations did not overlap')
            require((seq[2] < seq[3]) == (case % 2 == 1), 'wrong winner order')
            all_sequences.extend(seq)
            if case <= 2:
                require(one('defer-result').get('accepted') == str(case % 2), 'late deferral accepted')
            verified = one('verified')
            expected = {'terminal': 'ReplyDelivered' if case == 3 else 'PeerClosed', 'count': '1',
                        'duplicate': 'denied', 'late': 'denied', 'pending': '0', 'deferred': '0',
                        'queued': '0', 'waiters': '0', 'endpoint_caps': '0'}
            require(all(verified.get(k) == v for k, v in expected.items()), 'terminal or cleanup mismatch')
            require(len(selected) == (7 if case <= 2 else 6), 'unexpected case records')
        require(len(set(all_sequences)) == 16, 'duplicate sequence')
        require(len(set(endpoints)) == 4 and min(endpoints) >= 0, 'endpoint generation reused')
        require(generations == sorted(set(generations)) and min(generations) > 0, 'call generation reused')
        require(len(set(identities)) == 1, 'participant identity changed')
        finals = [r for r, line in zip(records, lines) if line.startswith('[IPC-RACE] PASS ')]
        require(len(finals) == 1, 'single final inventory required')
        final = finals[0]
        require(all(final.get(k) == v for k, v in {'cases': '4', 'cleanup': 'reaped',
                'scope': 'kernel-method', 'account_auth': 'NOT_RUN', 'real_exec': 'NOT_RUN',
                'numeric_pid_reuse': 'NOT_RUN'}.items()), 'final scope or lifecycle promotion')
        require(int(final['frames_before']) > 0 and final['frames_before'] == final['frames_after'],
                'frame cleanup drift')
        require(len(records) == 27, 'unexpected total records')
        require(raw.count('[IPC-RACE-LIFECYCLE] withdrawal=PASS ordinary=1 deferred=1 duplicate=denied counter=preserved real_exec=NOT_RUN') == 1,
                'exec withdrawal method receipt missing')
        require(raw.count('[IPC-RACE-CALLER] withdrawal=PASS stale=denied next_call=delivered real_exec=NOT_RUN') == 1,
                'caller withdrawal method receipt missing')
    except (KeyError, ValueError, IndexError, TypeError):
        errors.append('missing or malformed evidence')
    require(not any(m in raw for m in ('PANIC', 'panicked at', '[FAULT]', 'UNEXPECTED')), 'guest fault')
    return {'status': 'FAIL' if errors else 'PASS', 'errors': errors,
            'scope': 'concurrent native kernel-method race; authenticated Account peers and real exec/PID reuse NOT_RUN',
            'milestone': 'PARTIAL', 'production_fastlane': 'BLOCKED'}
