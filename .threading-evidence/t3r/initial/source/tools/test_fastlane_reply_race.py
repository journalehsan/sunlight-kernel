import unittest

from fastlane_reply_race_validate import assess


def fixture():
    lines = []
    for case in range(1, 5):
        lines.append(f'[IPC-RACE] case={case} event=identity caller=1 caller_inc=2 caller_vm=3 server=4 server_inc=5 server_vm=6 endpoint={case-1} generation={case} scope=kernel-method account_auth=NOT_RUN')
        first = (case - 1) * 4 + 1
        lines += [f'[IPC-RACE] seq={first} case={case} event=publish-enter cpu=0',
                  f'[IPC-RACE] seq={first+1} case={case} event=close-enter cpu=1']
        if case <= 2:
            lines.append(f'[IPC-RACE] case={case} event=defer-result accepted={case%2} cpu=0')
        pub, close = (first+2, first+3) if case % 2 else (first+3, first+2)
        lines += [f'[IPC-RACE] seq={pub} case={case} event=publish-finished cpu=0',
                  f'[IPC-RACE] seq={close} case={case} event=close-committed cpu=1',
                  f'[IPC-RACE] case={case} event=verified terminal={"ReplyDelivered" if case == 3 else "PeerClosed"} count=1 duplicate=denied late=denied pending=0 deferred=0 queued=0 waiters=0 endpoint_caps=0']
    lines += ['[IPC-RACE-LIFECYCLE] withdrawal=PASS ordinary=1 deferred=1 duplicate=denied counter=preserved real_exec=NOT_RUN',
              '[IPC-RACE-CALLER] withdrawal=PASS stale=denied next_call=delivered real_exec=NOT_RUN',
              '[IPC-RACE] PASS cases=4 frames_before=100 frames_after=100 cleanup=reaped scope=kernel-method account_auth=NOT_RUN real_exec=NOT_RUN numeric_pid_reuse=NOT_RUN']
    return '\n'.join(lines)


class RaceEvidenceTests(unittest.TestCase):
    def test_complete_evidence_is_scoped(self):
        result = assess(fixture(), 2)
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['milestone'], 'PARTIAL')
        self.assertEqual(result['production_fastlane'], 'BLOCKED')

    def test_every_record_required_exactly_once(self):
        for line in fixture().splitlines():
            with self.subTest(line=line):
                self.assertEqual(assess(fixture().replace(line, ''), 2)['status'], 'FAIL')
                self.assertEqual(assess(fixture() + '\n' + line, 2)['status'], 'FAIL')

    def test_sequential_or_same_cpu_is_not_overlap(self):
        for old, new in [('seq=2 case=1', 'seq=5 case=1'),
                         ('close-enter cpu=1', 'close-enter cpu=0'),
                         ('close-committed cpu=1', 'close-committed cpu=0'),
                         ('seq=3 case=1', 'seq=4 case=1')]:
            with self.subTest(old=old):
                self.assertEqual(assess(fixture().replace(old, new), 2)['status'], 'FAIL')
        self.assertEqual(assess(fixture(), 1)['status'], 'FAIL')

    def test_cleanup_identity_and_scope_are_required(self):
        for old, new in [('caller_inc=2', 'caller_inc=0'), ('server_vm=6', 'server_vm=0'),
                         ('pending=0', 'pending=1'), ('deferred=0', 'deferred=1'),
                         ('waiters=0', 'waiters=1'), ('endpoint_caps=0', 'endpoint_caps=1'),
                         ('frames_after=100', 'frames_after=99'), ('count=1', 'count=2'),
                         ('accepted=0', 'accepted=1'), ('account_auth=NOT_RUN', 'account_auth=PASS'),
                         ('real_exec=NOT_RUN', 'real_exec=PASS'),
                         ('numeric_pid_reuse=NOT_RUN', 'numeric_pid_reuse=PASS'),
                         ('generation=4', 'generation=3'), ('endpoint=3', 'endpoint=2'),
                         ('late=denied', 'late=accepted')]:
            with self.subTest(old=old):
                self.assertEqual(assess(fixture().replace(old, new), 2)['status'], 'FAIL')

    def test_faults_and_malformed_rows_fail(self):
        for marker in ['PANIC', '[FAULT]', 'UNEXPECTED', 'panicked at']:
            self.assertEqual(assess(fixture() + '\n' + marker, 2)['status'], 'FAIL')
        self.assertEqual(assess(fixture().replace('seq=1 ', 'seq=x '), 2)['status'], 'FAIL')
        self.assertEqual(assess(fixture().replace('seq=1 ', 'seq=1 seq=2 '), 2)['status'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
