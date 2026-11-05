import unittest

from fastlane_native_boundary_validate import CASES, assess, required


def fixture(cpus=2):
    return '\n'.join(required(cpus) + [
        f'[IPC-DEFER] case={case} result=PASS' for case in CASES
    ] + ['[IPC-DEFER] PASS cases=9 caller_inc=1 caller_vm=2 pending=0 deferred=0 queued=0 scope=boot-method concurrent_race=NOT_RUN'])


class EvidenceBoundaryTests(unittest.TestCase):
    def test_complete_scoped_evidence(self):
        self.assertEqual(assess(fixture(), 2)['status'], 'PASS')

    def test_every_case_is_required_once(self):
        for case in CASES:
            marker = f'[IPC-DEFER] case={case} result=PASS'
            with self.subTest(case=case):
                self.assertEqual(assess(fixture().replace(marker, ''), 2)['status'], 'FAIL')
                self.assertEqual(assess(fixture() + '\n' + marker, 2)['status'], 'FAIL')

    def test_no_promotion_to_concurrent_race(self):
        self.assertEqual(assess(fixture().replace('concurrent_race=NOT_RUN', 'concurrent_race=PASS'), 2)['status'], 'FAIL')

    def test_identity_cleanup_and_cpu_evidence_required(self):
        for old, new in [('caller_inc=1', 'caller_inc=0'), ('caller_vm=2', 'caller_vm=0'),
                         ('pending=0', 'pending=1'), ('deferred=0', 'deferred=1'),
                         ('queued=0', 'queued=1'), ('cases=9', 'cases=8')]:
            with self.subTest(old=old):
                self.assertEqual(assess(fixture().replace(old, new), 2)['status'], 'FAIL')
        self.assertEqual(assess(fixture(2), 4)['status'], 'FAIL')

    def test_missing_boot_regression_or_fault_fails(self):
        for marker in required(2):
            with self.subTest(marker=marker):
                self.assertEqual(assess(fixture().replace(marker, ''), 2)['status'], 'FAIL')
        for marker in ('PANIC', '[FAULT]', 'UNEXPECTED', '[SESSION-E3]', '[D36R-TEST]'):
            with self.subTest(marker=marker):
                self.assertEqual(assess(fixture() + '\n' + marker, 2)['status'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
