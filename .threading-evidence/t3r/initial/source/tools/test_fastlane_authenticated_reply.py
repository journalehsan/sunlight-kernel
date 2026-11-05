"""Parser boundary controls only; generated records are never native evidence."""
import unittest
from fastlane_authenticated_reply_validate import assess


def transcript():
    lines = ['[SESSION-E3] checkpoint=account-activated pid=20 incarnation=32 vm=51',
             '[SESSION-E3] checkpoint=account-activated pid=60 incarnation=72 vm=120']
    def user(r, event, pid=40):
        lines.append(f'[AUTH-REPLY] round={r} event={event} pid={pid}')
    def kernel(r, event, pid=40, inc=52, vm=80, auth=0, deferred=0):
        lines.append(f'[AUTH-KERNEL] round={r} event={event} pid={pid} inc={inc} vm={vm} '
                     f'uid=0 gid=0 auth={auth} pending=0 outcome=0 reply=0 deferred={deferred} cpu=1')
    for r in range(1, 16):
        vm = 80 + (r - 1) // 5
        outcome = 'ReplyDelivered' if r % 5 == 3 else 'PeerClosed'
        user(r, 'foreign-owner-and-send-only-denied', 20)
        kernel(r, 'received', vm=vm)
        lines.append(f'[AUTH-BINDING] round={r} server=40 server_inc=52 server_vm={vm} '
                     f'caller=20 caller_inc=32 caller_vm=51 caller_auth=1 endpoint={100+r} generation={r*4} live=1')
        if r > 1:
            user(r, 'prior-token-denied-with-new-pending')
        if r == 15:
            kernel(r, 'shutdown-enter', vm=vm, deferred=1)
        elif r % 5 == 0:
            kernel(r, 'exec-enter', vm=vm, deferred=1)
        lines.append(f'[AUTH-TERMINAL] round={r} caller=20 caller_inc=32 caller_vm=51 '
                     f'endpoint={100+r} generation={r*4} outcome={outcome} cpu=1')
        if r == 15:
            user(r, 'shutdown-outstanding')
        elif r % 5 == 0:
            kernel(r, 'exec-return', vm=vm+1)
            user(r, 'exec-stale-denied-fresh-delivered')
        else:
            user(r, {1: 'defer-before-close', 2: 'close-before-defer-denied',
                     3: 'complete-before-close', 4: 'close-before-complete-denied'}[r % 5])
            user(r, 'fresh-delivered-after-late-denial')
        user(r, outcome, 20)
        kernel(r, 'consumed', pid=20, inc=32, vm=51, auth=1)
        user(r, 'retired-endpoint-denied', 20)
    user(0, 'peer-exit')
    user(0, 'foreign-live-completion-denied')
    user(0, 'foreign-target-still-live', 41)
    user(0, 'account-rounds-complete', 20)
    user(0, 'peer-reaped', 41)
    lines += ["[SCHED] process_mark_finished pid=40 name='reply-peer' reason=exit code=0",
              "[SCHED] process_reaped pid=40 name='reply-peer'",
              '[SESSION-E3] checkpoint=fixture-ipc-cleaned pid=40 owned_endpoints=0 pending_call=none '
              'reply_target=none deferred_targets=0 pending_commits=0']
    return '\n'.join(lines)


class EvidenceTests(unittest.TestCase):
    def reject(self, raw):
        result = assess(raw, 2)
        self.assertEqual(result['status'], 'FAIL', result)

    def test_scoped_success_cannot_close_independent_gates(self):
        result = assess(transcript(), 2)
        self.assertEqual(result['status'], 'PASS', result)
        self.assertEqual(result['milestone'], 'PARTIAL')
        for gate in ('numeric_pid_reuse', 'concurrent_independent_closer', 'grant_mapping_reclamation'):
            self.assertEqual(result[gate], 'BLOCKED')
        self.assertEqual(result['privileged_image_role_exec'], 'NOT_RUN')

    def test_guest_success_without_kernel_transition_is_rejected(self):
        for prefix in ('[AUTH-BINDING]', '[AUTH-TERMINAL]', '[AUTH-KERNEL]'):
            self.reject('\n'.join(l for l in transcript().splitlines() if not l.startswith(prefix)))

    def test_each_required_record_is_necessary(self):
        lines = transcript().splitlines()
        for i in range(len(lines)):
            self.reject('\n'.join(lines[:i] + lines[i+1:]))

    def test_duplicate_terminal_or_duplicate_field_is_rejected(self):
        raw = transcript()
        self.reject(raw + '\n' + next(l for l in raw.splitlines() if l.startswith('[AUTH-TERMINAL]')))
        self.reject(raw.replace('caller_auth=1', 'caller_auth=1 caller_auth=0', 1))

    def test_exact_process_image_endpoint_and_generation_are_required(self):
        for key in ('server_inc', 'server_vm', 'caller', 'caller_inc', 'caller_vm', 'endpoint', 'generation'):
            raw = transcript()
            line = next(l for l in raw.splitlines() if l.startswith('[AUTH-BINDING]'))
            value = next(f for f in line.split() if f.startswith(key+'='))
            self.reject(raw.replace(line, line.replace(value, key+'=999'), 1))
        self.reject(transcript().replace('caller_auth=1', 'caller_auth=0'))
        self.reject(transcript().replace('live=1', 'live=0'))

    def test_no_exec_or_unchanged_vm_or_remaining_authority_is_rejected(self):
        raw = transcript()
        self.reject(raw.replace('event=exec-return', 'event=simulated-return'))
        self.reject(raw.replace('event=exec-return pid=40 inc=52 vm=81',
                                'event=exec-return pid=40 inc=52 vm=80'))
        self.reject(raw.replace('event=exec-enter pid=40 inc=52 vm=80',
                                'event=exec-enter pid=40 inc=53 vm=80'))
        self.reject(raw.replace('deferred=1', 'deferred=0'))

    def test_wrong_terminal_cpu_or_unconsumed_result_is_rejected(self):
        self.reject(transcript().replace('cpu=1', 'cpu=2'))
        self.reject(transcript().replace('outcome=PeerClosed', 'outcome=ReplyDelivered', 1))
        self.reject(transcript().replace('pending=0 outcome=0', 'pending=0 outcome=1'))
        self.reject(transcript() + '\n[SESSION-E3-CLIENT] checkpoint=FAIL')


if __name__ == '__main__':
    unittest.main()
