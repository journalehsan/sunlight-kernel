"""Labelled synthetic parser controls; never native execution evidence."""
import unittest
from fastlane_m4_assess import assess
IDENTITY = {k: True for k in ("source", "elf", "iso", "services")}


def synthetic():
    expected = {'start': [0], 'unauthorized-stolen-stale': [0], 'notify-before-wait': [1],
                'sleeping-repeated': [9], 'registration-race': [1, 2],
                'cancel-notify-orders': [0, 1], 'signal-cancel-notify': [5, 6], 'consumer-killed-wait': [0], 'revocation-wait': [0], 'producer-exit-wait': [0],
                'consumer-exit-notify': [3, 4], 'sequence-exhaustion': [0],
                'reclaimed-cycle': list(range(16)), 'exec-authority-retired': [0], 'complete': [0]}
    lines = [f'[FL-M4] case={case} detail={detail} generation={30 if case == 'consumer-killed-wait' else 1} pid=10' for case, ds in expected.items() for detail in ds]
    lines += [f'[FL-BIND] op=3 state=3 generation={g} producer=10 consumer=11 producer_vm=1 consumer_vm=2 issuer_inc=1 epoch=1 producer_inc=1 consumer_inc=2' for g in range(1, 31)]
    for _ in range(32):
        lines += ['[FL-BELL] event=registered generation=1 consumer=11 inc=2 vm=2 cpu=1',
                  '[FL-BELL] event=complete generation=1 consumer=11 inc=2 vm=2 cpu=0 blocked=1 result=0 sequence=1',
                  '[FL-BELL] event=take generation=1 consumer=11 inc=2 vm=2 cpu=1 result=0 sequence=1']
    for mode in range(1, 7):
        result = {1: (0, 0), 2: (10, 0), 3: (0, 4), 4: (0, 0), 5: (0, 0), 6: (0, 0)}[mode]
        lines += [f'[FL-BELL-RACE] event=entered mode={mode} actor=1 cpu=1',
                  f'[FL-BELL-RACE] event=entered mode={mode} actor=2 cpu=0',
                  f'[FL-BELL-RACE] event=lock-contended mode={mode} actor=2 cpu=0',
                  f'[FL-BELL-RACE] event=returned mode={mode} actor=1 result={result[0]}',
                  f'[FL-BELL-RACE] event=returned mode={mode} actor=2 result={result[1]}']
    lines += ['[FL-BELL] event=registered generation=30 consumer=11 inc=2 vm=2 cpu=1',
              '[SCHED] process_reaped pid=11 name=session-racer',
              '[FL-BELL-RESOURCE] generation=1 rows=1 capacity=8 active=0 waiters=0 receipts=0 pending=0']
    lines += ['[FL-BELL-PARK] generation=1 parked=1 owner=255 queued=255 pins=0 ready=0'] * 32
    return '\n'.join(lines)


class Controls(unittest.TestCase):
    def test_synthetic_control(self):
        self.assertEqual(assess(synthetic(), 2, IDENTITY)['status'], 'PASS')

    def test_reject_missing_evidence(self):
        for marker in ('case=complete', 'event=registered', 'event=take', 'event=lock-contended', '[FL-BIND]'):
            with self.subTest(marker=marker):
                raw = '\n'.join(s for s in synthetic().splitlines() if marker not in s)
                self.assertEqual(assess(raw, 2, IDENTITY)['status'], 'FAIL')

    def test_reject_bad_identity_same_cpu_and_double_completion(self):
        for raw in (synthetic().replace('consumer_vm=2', 'consumer_vm=1'),
                    synthetic().replace('cpu=1', 'cpu=0'),
                    synthetic() + '\n[FL-BELL] event=complete generation=1',
                    synthetic() + '\nPANIC', synthetic().replace('blocked=1', 'blocked=0'),
                    synthetic().replace('pins=0', 'pins=1')):
            self.assertEqual(assess(raw, 2, IDENTITY)['status'], 'FAIL')

    def test_missing_identity_and_failed_attempt(self):
        self.assertEqual(assess(synthetic(), 2)['status'], 'FAIL')
        self.assertEqual(assess(synthetic(), 2, IDENTITY, {'failure': 'timeout'})['status'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
