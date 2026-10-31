"""Synthetic parser negatives; these never count as native execution evidence."""
import unittest
from fastlane_f11b0h4_validate import assess, host_ack_errors, reaper_userspace_errors

IDENTITY = dict.fromkeys(('iso_matches', 'elf_matches', 'services_match', 'source_matches'), True)


def transcript():
    lines = []
    for name, pid, result in (('request-start', 39, 0), ('controller-ready', 40, 0),
            ('policy-accepted-rendezvous', 40, 0), ('old-call-peer-closed', 39, 0),
            ('manager-restart-response', 40, 0), ('replacement-old-session-denied', 39, 11),
            ('fixture-complete', 39, 0)):
        lines.append(f'[SESSION-E3-CLIENT] case=N4 checkpoint={name} pid={pid} uid=0 result={result} fixture=123')
    lines.append('[H4-CTL] fixture=123 checkpoint=ready pid=41 target=18 route=status-ok')
    for pid in (18, 39, 40, 41):
        lines.append(f"[SCHED] process_reaped pid={pid} name='fixture'")
    for pid in (39, 40, 41):
        lines.append(f'[SESSION-E3] checkpoint=fixture-ipc-cleaned pid={pid} owned_endpoints=0 pending_call=none reply_target=none deferred_targets=0 pending_commits=0')
    lines.append("[SCHED] process_mark_finished pid=39 name='session-race-probe' reason=exit code=0")
    lines += ['[SESSION-E3] checkpoint=account-activated pid=18 incarnation=30 vm=47',
              '[SESSION-E3] checkpoint=account-activated pid=42 incarnation=54 vm=80',
              '[SESSION-E3] checkpoint=registered pid=26 sessiond=1 session=1 epoch=1',
              '[SESSION-E3] checkpoint=login-proof-committed pid=18 incarnation=30 vm=47 account=1',
              '[SESSION-E3] checkpoint=request-binding recipient=39 recipient_inc=51 request_generation=4',
              '[SESSION-E3] checkpoint=inventory sessions=0 receipts=0 account_grants=0 pending_account_deliveries=0']
    events = [(60,0,41,53,123,18,0), (1,1,18,30,4,1,1), (65,1,18,30,1,1,39),
              (61,0,41,53,123,18,0), (62,0,11,23,123,18,41), (64,0,11,23,18,15,1),
              (2,1,0,0,1,1,0), (3,1,18,30,1,1,0), (4,1,18,30,1,1,0),
              (70,1,18,30,1,0,0), (71,1,18,30,1,0,1), (10,1,18,30,1,47,0),
              (20,1,18,30,1,1,18), (11,1,18,30,15,1,1), (30,1,39,51,15,4,0),
              (80,0,39,51,123,4,0)]
    lines.append(f'[H3-TRACE] schema=1 count={len(events)} capacity=256 overflow=0')
    for seq, event in enumerate(events, 1):
        e, cpu, pid, inc, a, b, c = event
        lines.append(f'[H3-TRACE] schema=1 seq={seq} event={e} cpu={cpu} pid={pid} inc={inc} a={a} b={b} c={c}')
    return '\n'.join(lines)


class CompletionTests(unittest.TestCase):
    def check_bad(self, raw, **kwargs):
        result = assess(raw, 'N4', 2, '123', identity=kwargs.pop('identity', IDENTITY), **kwargs)
        self.assertEqual(result['classification'], 'FORCED_AND_FAILED', result)
        self.assertFalse(result['host_terminal_acknowledged'])

    def test_complete_guest_requires_separate_host_ack_and_never_implies_contention(self):
        result = assess(transcript(), 'N4', 2, '123', identity=IDENTITY)
        self.assertEqual(result['terminal_protocol'], 'GUEST_VALIDATED_PENDING_HOST_ACK', result)
        self.assertFalse(result['host_terminal_acknowledged'])
        self.assertEqual(result['genuine_reaper_contention'], 'NOT_FORCED')

    def test_completion_marker_alone(self):
        self.check_bad('[SESSION-E3-CLIENT] case=N4 checkpoint=fixture-complete pid=39 uid=0 result=0 fixture=123')

    def test_required_assertions_and_cleanup(self):
        for marker in ('checkpoint=old-call-peer-closed', 'checkpoint=replacement-old-session-denied',
                       'process_reaped pid=41', 'checkpoint=fixture-ipc-cleaned pid=39', 'event=64', 'event=20'):
            self.check_bad('\n'.join(line for line in transcript().splitlines() if marker not in line))

    def test_timeout_truncation_overflow_holes_stale_wrong_identity(self):
        raw = transcript()
        for broken in (raw.replace('overflow=0', 'overflow=1'), raw.replace('event=20', 'event=999'),
                       raw.replace('seq=5 ', 'seq=6 '), raw.rsplit('\n', 1)[0],
                       raw.replace('fixture=123', 'fixture=122'), raw.replace('case=N4', 'case=N6'),
                       raw.replace('count=16', 'count=17'), raw + '\n[SESSION-E3-CLIENT] checkpoint=FAIL',
                       raw.replace('cpu=1', 'cpu=0'), raw.replace('result=0 fixture=123', 'result=0 result=1 fixture=123')):
            self.check_bad(broken)
        self.check_bad(raw, identity={**IDENTITY, 'source_matches': False})
        self.check_bad(raw, failure={'type': 'TimeoutError'})
        self.check_bad(raw.replace('inc=30 a=4 b=1', 'inc=30 a=6 b=1'))

    def test_ready_and_manager_acceptance_are_distinct(self):
        raw = '\n'.join(line for line in transcript().splitlines() if 'event=62' not in line)
        self.check_bad(raw)

    def test_host_ack_requires_exact_fixture_and_artifact_identity(self):
        expected = dict(fixture='123', case='N4', cpus=2, serial_sha256='serial', iso_sha256='iso', elf_sha256='elf')
        self.assertTrue(host_ack_errors(None, **expected))
        ack = {**expected, 'security_assertions': 'PASS', 'cleanup': 'PASS', 'identity': IDENTITY}
        self.assertFalse(host_ack_errors(ack, **expected))
        for key, value in expected.items():
            self.assertTrue(host_ack_errors({**ack, key: 'wrong'}, **expected))
        self.assertTrue(host_ack_errors({**ack, 'cleanup': 'NOT_RUN'}, **expected))
        self.assertTrue(host_ack_errors({**ack, 'identity': {}}, **expected))


class UserspaceReaperTests(unittest.TestCase):
    def evidence(self):
        return '\n'.join([
            '[REAP-TEST] fixture=123 checkpoint=issuer-userspace-ready issuer=18 observer=40',
            '[SESSION-E3] checkpoint=mint-committed pid=18 incarnation=30 result=0',
            '[SESSION-E3] checkpoint=inventory receipts=1 account_grants=1 pending_account_deliveries=1 observed_mints=1 observed_consumes=0',
            "[SCHED] process_mark_finished pid=18 name='uac_service' reason=pending-fatal-signal code=143",
            '[H3-TRACE] seq=1 event=4 pid=18 inc=30 cpu=1 a=1 b=1 c=0',
            '[H3-TRACE] seq=2 event=1 pid=18 inc=30 cpu=1 a=6 b=1 c=1',
            '[H3-TRACE] seq=3 event=10 pid=18 inc=30 cpu=0 a=2 b=47 c=0',
            '[H3-TRACE] seq=4 event=11 pid=18 inc=30 cpu=0 a=15 b=2 c=1',
            '[H3-TRACE] seq=5 event=30 pid=39 inc=51 cpu=0 a=15 b=4 c=0',
            '[H3-TRACE] seq=6 event=20 pid=18 inc=30 cpu=0 a=1 b=1 c=18',
            '[SESSION-E3] checkpoint=inventory receipts=0 account_grants=0 pending_account_deliveries=0 observed_mints=1 observed_consumes=0',
            '[SESSION-E3-CLIENT] case=N6 fixture=123 checkpoint=userspace-readiness-cleaned result=0',
        ])

    def test_resource_reap_after_committed_grant(self):
        # This helper checks only the additional mode; it cannot certify the
        # base fixture, source identity, cleanup or host acknowledgement alone.
        self.assertFalse(reaper_userspace_errors(self.evidence(), '123'))
        self.assertTrue(assess(self.evidence(), 'N6', 2, '123', identity=IDENTITY)['errors'])

    def test_reject_missing_security_path_and_readiness_cleanup(self):
        raw = self.evidence()
        for marker in ('[REAP-TEST]', 'checkpoint=mint-committed', 'pending-fatal-signal',
                       'event=4 ', 'event=10', 'event=11', 'event=30', 'event=20',
                       'checkpoint=userspace-readiness-cleaned'):
            broken = '\n'.join(line for line in raw.splitlines() if marker not in line)
            self.assertTrue(reaper_userspace_errors(broken, '123'), marker)

    def test_reject_wrong_case_identity_endpoint_order_and_resource_counts(self):
        raw = self.evidence()
        for broken in (raw.replace('issuer=18', 'issuer=19'), raw.replace('fixture=123', 'fixture=122'),
                       raw.replace('a=6 b=1', 'a=4 b=1'), raw.replace('a=2 b=47', 'a=1 b=47'),
                       raw.replace('a=15 b=2', 'a=16 b=2'), raw.replace('seq=5 event=30', 'seq=7 event=30'),
                       raw.replace('seq=1 event=4', 'seq=7 event=4'),
                       raw + '\n[H3-TRACE] seq=7 event=4 pid=18 inc=30 cpu=1 a=1 b=1 c=0',
                       raw.replace('observed_mints=1', 'observed_mints=2'),
                       raw.replace('pending_account_deliveries=1', 'pending_account_deliveries=0'),
                       raw.replace('event=30 pid=39 inc=51 cpu=0', 'event=30 pid=39 inc=51 cpu=1')):
            self.assertTrue(reaper_userspace_errors(broken, '123'), broken)


if __name__ == '__main__':
    unittest.main()
