"""Fail-closed controls for incomplete accounting evidence, not native proofs."""
import unittest
from fastlane_m53r51_accounting import assess


def checkpoint(label, free):
    lines = [f'[R51-PMM] label={label} managed=100 free={free} used={100-free} scope=pmm-lock-snapshot']
    lines += [f'[R51-CLASS] label={label} class={i} frames=0 scope=primary-class-overlaps-owner-and-quota' for i in range(16)]
    return '\n'.join(lines)


class AccountingEvidenceControls(unittest.TestCase):
    def test_no_evidence(self):
        self.assertEqual(assess('')['status'], 'FAIL')

    def test_nonzero_drift_stays_unattributed(self):
        result = assess(checkpoint('baseline', 90) + '\n' + checkpoint('quiescent', 84))
        self.assertEqual(result['delta']['free'], -6)
        self.assertEqual(result['attribution'], 'UNATTRIBUTED')
        self.assertFalse(result['global_zero_drift_qualified'])

    def test_zero_drift_does_not_prove_lifetimes(self):
        result = assess(checkpoint('baseline', 90) + '\n' + checkpoint('quiescent', 90))
        self.assertEqual(result['status'], 'OBSERVED')
        self.assertFalse(result['global_zero_drift_qualified'])

    def test_missing_class(self):
        raw = checkpoint('baseline', 90) + '\n' + checkpoint('quiescent', 90)
        self.assertEqual(assess('\n'.join(raw.splitlines()[:-1]))['status'], 'FAIL')

    def test_duplicate_class(self):
        raw = checkpoint('baseline', 90) + '\n' + checkpoint('quiescent', 90)
        self.assertEqual(assess(raw + '\n' + raw.splitlines()[-1])['status'], 'FAIL')

    def test_reversed_checkpoints(self):
        self.assertEqual(assess(checkpoint('quiescent', 90) + '\n' + checkpoint('baseline', 90))['status'], 'FAIL')

    def test_duplicate_baseline(self):
        raw = checkpoint('baseline', 90) + '\n' + checkpoint('baseline', 90) + '\n' + checkpoint('quiescent', 90)
        self.assertEqual(assess(raw)['status'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
