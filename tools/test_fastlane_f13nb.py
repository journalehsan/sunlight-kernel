"""Evidence boundaries from the failed D3 replay must survive parser changes."""
import unittest

from fastlane_f13nb_validate import d3_checkpoints
from fastlane_f13nb_resident import required, validate_resident


class D3EvidenceTests(unittest.TestCase):
    def completed(self):
        return '\n'.join([f'[D{i}-TEST] COMPLETE PASS' for i in (1, 2, 3)] +
                         [f'[D3-MEM] cycle={i} after_free=100' for i in range(6)])

    def test_guest_assertion_overrides_completion(self):
        raw = self.completed() + '\n[D2-TEST] driver FAIL panicked at d3_driver.rs:50'
        self.assertEqual(d3_checkpoints(raw, exit_code=1)['classification'], 'GUEST_ASSERTION_FAILED')

    def test_missing_trigger_is_not_assertion_failure(self):
        self.assertEqual(d3_checkpoints('[D1-TEST] COMPLETE PASS', exit_code=1)['classification'],
                         'D3_NOT_REACHED')

    def test_watchdog_is_distinct(self):
        self.assertEqual(d3_checkpoints('[D3-TEST] began', '2 CPU timed out', 1)['classification'],
                         'HOST_WATCHDOG_TIMEOUT')

    def test_truncated_log_cannot_pass_from_exit_code(self):
        self.assertEqual(d3_checkpoints('[D3-TEST] began', exit_code=0)['classification'],
                         'INCOMPLETE_EVIDENCE')

    def test_duplicate_cycles_cannot_pass(self):
        raw = self.completed().replace('cycle=5', 'cycle=4')
        self.assertNotEqual(d3_checkpoints(raw, exit_code=0)['classification'], 'PASSED')

    def test_parser_failure_after_guest_completion_is_not_guest_failure(self):
        self.assertEqual(d3_checkpoints(self.completed(), exit_code=1)['classification'],
                         'HARNESS_FAILED_AFTER_COMPLETION')

    def test_cleanup_and_drift_require_samples(self):
        row = d3_checkpoints(self.completed(), exit_code=0)
        self.assertEqual(row['classification'], 'PASSED')
        self.assertEqual(row['steady_frame_range'], 0)
        self.assertEqual(d3_checkpoints('')['steady_frame_range'], None)

    def test_resident_rejects_test_manager_even_with_all_boot_markers(self):
        raw = '\n'.join(required(2)) + '\n[SUNLIGHTD] capability profile d2-driver uid=0'
        row = validate_resident(raw, 2)
        self.assertFalse(row['passed'])
        self.assertTrue(row['test_fixture_markers'])

    def test_resident_needs_actual_service_startup(self):
        raw = '\n'.join(m for m in required(2) if 'spawned sunlight-tls' not in m)
        self.assertFalse(validate_resident(raw, 2)['passed'])

    def test_resident_complete_normal_startup(self):
        self.assertTrue(validate_resident('\n'.join(required(2)), 2)['passed'])


if __name__ == '__main__':
    unittest.main()
