import unittest

from fastlane_f13nr2v_validate import validate_native


class NativeEvidenceTests(unittest.TestCase):
    def trace(self, cpus=2):
        mask = ((1 << cpus) - 1) & ~1
        lines = [f'[R2V] cpu cpu={cpu} apic={cpu}' for cpu in range(cpus)]
        for round_id in range(4):
            for cpu in range(1, cpus):
                lines.append(f'[R2V] waiter round={round_id} cpu={cpu} registered_mask={mask:#x} rejected=1 ack=1 accepted=1 handled=1 pass=1 sends=1 requests=1')
            lines.append(f'[R2V] cycle round={round_id} source=0 execution={cpus - 1} rejected={mask:#x} released={mask:#x} timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped')
        lines.append(f'[R2V] PASS cpus={cpus} rounds=4 forced={4 * (cpus - 1)} frames_before=100 frames_after=100 metadata_slots=4 retired=4 timer_assistance=0')
        return '\n'.join(lines) + '\n'

    def test_complete_matrix(self):
        for cpus in [1, 2, 4, 12]:
            self.assertEqual(validate_native(self.trace(cpus), cpus)['status'], 'pass')

    def test_boot_only_is_not_run(self):
        self.assertEqual(validate_native('Phase 1 OK\n', 2)['status'], 'not_run')

    def test_panic_and_wrong_physical_cpu_fail(self):
        self.assertEqual(validate_native('[KERNEL PANIC] before completion\n', 2)['status'], 'fail')
        self.assertEqual(validate_native(self.trace().replace('cpu=1 apic=1', 'cpu=1 apic=0'), 2)['status'], 'fail')

    def test_missing_rejection_or_delivery_fails(self):
        for original, changed in [('rejected=1', 'rejected=0'), ('accepted=1', 'accepted=0'),
                                  ('handled=1', 'handled=0'), ('pass=1', 'pass=0'),
                                  ('registered_mask=0x2', 'registered_mask=0x0'),
                                  ('execution=1', 'execution=0'), ('frames_after=100', 'frames_after=99'),
                                  ('timer_assistance=0', 'timer_assistance=1')]:
            self.assertEqual(validate_native(self.trace().replace(original, changed), 2)['status'], 'fail')

    def test_duplicate_or_missing_round_fails(self):
        raw = self.trace()
        self.assertEqual(validate_native(raw + raw.splitlines()[3] + '\n', 2)['status'], 'fail')
        self.assertEqual(validate_native(raw.replace('round=3', 'round=2'), 2)['status'], 'fail')


if __name__ == '__main__':
    unittest.main()
