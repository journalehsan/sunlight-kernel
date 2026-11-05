"""Strict controls for T1 guest verdicts; synthetic logs are never native proof."""
import unittest
from threading_t1_validate import assess


class Verdict(unittest.TestCase):
    def test_complete(self):
        self.assertTrue(assess('[T1] PASS cpus=2\n', ['[T1] PASS']))

    def test_incomplete_and_failure_controls(self):
        for text, markers in [('', ['[T1] PASS']), ('[T1] PASS', ['[T1] PASS']),
                              ('[T1] PASS\n', ['[T1] PASS', '[SCHED-B] PASS']),
                              ('[T1] PASS\n[T1] PASS\n', ['[T1] PASS']),
                              ('[T1] PASS\n', []), ('[T1] PASS\n', [''])]:
            with self.subTest(text=text, markers=markers):
                self.assertFalse(assess(text, markers))
        for failure in ('KERNEL PANIC', '[EXCEPTION]', 'THREAD_PROBE FAIL',
                        'FP isolation FAILED', 'probe panicked', 'parser/raster FAILED'):
            self.assertFalse(assess('[T1] PASS\n' + failure, ['[T1] PASS']))


if __name__ == '__main__':
    unittest.main()
