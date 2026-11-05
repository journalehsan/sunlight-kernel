"""Authenticated userspace ordering/exec evidence; no concurrent-race promotion.

The surrounding N6/H4 assessment and artifact identity/host receipt are required
separately. Synthetic transcripts never constitute native validation.
"""
import re


def assess(raw, cpus):
    errors = []

    def require(ok, why):
        if not ok:
            errors.append(why)

    def rows(prefix):
        result = []
        for line in raw.splitlines():
            if line.startswith(prefix):
                pairs = re.findall(r'(\w+)=(\S+)', line)
                row = dict(pairs)
                require(len(pairs) == len(row), 'duplicate observation field')
                result.append(row)
        return result

    users = rows('[AUTH-REPLY] ')
    kernels = rows('[AUTH-KERNEL] ')
    bindings = rows('[AUTH-BINDING] ')
    terminals = rows('[AUTH-TERMINAL] ')
    observations = rows('[SESSION-E3] ')

    def one(records, **expected):
        selected = [r for r in records if all(r.get(k) == str(v) for k, v in expected.items())]
        require(len(selected) == 1, 'require one ' + str(expected))
        return selected[0]

    try:
        activated = [r for r in observations if r.get('checkpoint') == 'account-activated']
        require(len(activated) == 2, 'old and replacement Account activations missing')
        account = activated[0]
        endpoints, generations, identities, images = [], [], [], []
        exec_images = {}
        for round in range(1, 16):
            binding = one(bindings, round=round)
            received = one(kernels, round=round, event='received')
            consumed = one(kernels, round=round, event='consumed')
            terminal = one(terminals, round=round)
            outcome = 'ReplyDelivered' if round % 5 == 3 else 'PeerClosed'
            for key, source in (('caller', 'pid'), ('caller_inc', 'incarnation'), ('caller_vm', 'vm')):
                require(binding[key] == account[source], 'caller is not exact authenticated Account')
            require(binding['caller_auth'] == binding['live'] == '1', 'live Account authority missing')
            for key, source in (('server', 'pid'), ('server_inc', 'inc'), ('server_vm', 'vm')):
                require(binding[key] == received[source], 'server identity mismatch')
            require(binding['server'] != binding['caller'], 'independent userspace peers required')
            require(all(int(binding[k]) > 0 for k in ('server', 'server_inc', 'server_vm',
                    'caller', 'caller_inc', 'caller_vm', 'generation')), 'zero identity or call generation')
            require(all(terminal[k] == binding[k] for k in ('caller', 'caller_inc', 'caller_vm',
                    'endpoint', 'generation')), 'terminal belongs to another operation')
            require(terminal['outcome'] == outcome, 'wrong native terminal winner')
            require(consumed['pid'] == binding['caller'] and consumed['inc'] == binding['caller_inc']
                    and consumed['vm'] == binding['caller_vm'] and consumed['auth'] == '1',
                    'result consumed by wrong process or image')
            require(all(consumed[k] == '0' for k in ('pending', 'outcome', 'reply')),
                    'terminal state remained after consumption')
            require(received['uid'] == received['gid'] == received['auth'] == '0',
                    'ordinary root peer unexpectedly has image authority')
            for event in ('foreign-owner-and-send-only-denied', outcome, 'retired-endpoint-denied'):
                require(one(users, round=round, event=event)['pid'] == binding['caller'],
                        'wrong userspace result observer')
            if round > 1:
                require(one(users, round=round, event='prior-token-denied-with-new-pending')['pid']
                        == binding['server'], 'stale completion was not rejected by its own server')
            if round % 5:
                action = {1: 'defer-before-close', 2: 'close-before-defer-denied',
                          3: 'complete-before-close', 4: 'close-before-complete-denied'}[round % 5]
                for event in (action, 'fresh-delivered-after-late-denial'):
                    require(one(users, round=round, event=event)['pid'] == binding['server'],
                            'wrong server operation observer')
            elif round != 15:
                before = one(kernels, round=round, event='exec-enter')
                after = one(kernels, round=round, event='exec-return')
                require(all(before[k] == after[k] == received[k] for k in ('pid', 'inc', 'uid', 'gid', 'auth')),
                        'exec changed process identity or process credentials')
                require(before['vm'] == binding['server_vm'] and before['vm'] != after['vm'],
                        'real replacement VM not observed')
                exec_images[round] = after['vm']
                require(before['deferred'] == '1' and after['deferred'] == '0',
                        'old image reply authority survived exec')
                require(all(after[k] == '0' for k in ('pending', 'outcome', 'reply')),
                        'replacement inherited old IPC result state')
                require(one(users, round=round, event='exec-stale-denied-fresh-delivered')['pid']
                        == binding['server'], 'post-exec stale denial and inherited endpoint reply missing')
            else:
                shutdown = one(kernels, round=round, event='shutdown-enter')
                require(shutdown['pid'] == binding['server'] and shutdown['deferred'] == '1'
                        and shutdown['vm'] == binding['server_vm'], 'shutdown had no deferred Account reply')
                require(one(users, round=round, event='shutdown-outstanding')['pid'] == binding['server'],
                        'userspace shutdown entry missing')
            endpoints.append(int(binding['endpoint']))
            generations.append(int(binding['generation']))
            identities.append((binding['server'], binding['server_inc']))
            images.append(binding['server_vm'])
        require(len(set(endpoints)) == 15, 'endpoint lifetime identity reused')
        require(generations == sorted(set(generations)), 'call generation reused')
        require(len(set(identities)) == 1, 'peer process incarnation changed during exec')
        require(len(set(images + list(exec_images.values()))) == 3, 'replacement VM generation reused')
        for i in range(1, 15):
            require(images[i] == (exec_images[i] if i % 5 == 0 else images[i - 1]),
                    'image changed without the recorded exec transition')
        worker = identities[0][0]
        require(one(users, round=0, event='foreign-live-completion-denied')['pid'] == worker,
                'other server live completion authority was not rejected')
        require(one(users, round=0, event='foreign-target-still-live')['pid']
                == one(users, round=0, event='peer-reaped')['pid'], 'foreign completion lost its live target')
        require(one(users, round=0, event='peer-exit')['pid'] == worker, 'wrong peer exit')
        require(one(users, round=0, event='account-rounds-complete')['pid'] == account['pid'],
                'Account did not finish all operations')
        one(users, round=0, event='peer-reaped')
        require(bool(re.search(r"process_reaped pid=" + re.escape(worker) + r" name='", raw)),
                'actual worker reaping missing')
        require(bool(re.search(r"process_mark_finished pid=" + re.escape(worker)
                + r" name='[^']+' reason=exit code=0", raw)), 'worker did not exit zero')
        cleanup = one(observations, checkpoint='fixture-ipc-cleaned', pid=worker)
        require(all(cleanup.get(k) == v for k, v in {'owned_endpoints': '0', 'pending_call': 'none',
                'reply_target': 'none', 'deferred_targets': '0', 'pending_commits': '0'}.items()),
                'worker IPC metadata cleanup incomplete')
        require(len(bindings) == len(terminals) == 15 and len(kernels) == 35,
                'missing, duplicate, or unexpected native operation records')
        require(len(users) == 91, 'missing, duplicate, or unexpected userspace records')
        for r in kernels + terminals:
            require(0 <= int(r['cpu']) < cpus, 'native CPU outside configured topology')
        # Order is native event order, never cross-CPU clock subtraction.
        for round in range(1, 16):
            positions = [raw.index(f'{prefix} round={round} {event}') for prefix, event in (
                ('[AUTH-BINDING]', 'server='), ('[AUTH-TERMINAL]', 'caller='),
                ('[AUTH-KERNEL]', 'event=consumed'))]
            require(positions == sorted(positions), 'result observed before native terminal transition')
    except (ValueError, KeyError, IndexError, TypeError):
        errors.append('missing or malformed authenticated reply evidence')
    require(not any(s in raw for s in ('PANIC', 'panicked at', 'checkpoint=FAIL', '[FAULT]')),
            'guest failure')
    return {'status': 'FAIL' if errors else 'PASS', 'errors': errors,
            'scope': 'authenticated Account caller; owner-ordered userspace endpoint closure; real peer exec',
            'rounds': 15, 'exec_replacements': 2, 'peer_shutdown_with_pending_reply': 'PASS' if not errors else 'FAIL',
            'concurrent_independent_closer': 'BLOCKED',
            'privileged_image_role_exec': 'NOT_RUN', 'numeric_pid_reuse': 'BLOCKED',
            'grant_mapping_reclamation': 'BLOCKED', 'milestone': 'PARTIAL'}
