import unittest
from test_fastlane_f13nr2v import NativeEvidenceTests
from fastlane_f13nr2c_validate import validate_native


class OrderingEvidenceTests(unittest.TestCase):
    def trace(self, cpus=2):
        raw = NativeEvidenceTests().trace(cpus).replace('rounds=4', 'rounds=6').replace('retired=4', 'retired=6').replace('metadata_slots=4', 'metadata_slots=1')
        for round_id in range(6):
            owner = (1 if round_id >= 4 else cpus - 1) if cpus > 1 else 0
            path = 'terminate_current_user_process' if round_id == 3 else 'process_exit'
            raw += f'[R2C] reuse round={round_id} task=0 old_generation={round_id} generation={round_id+1} old_wake=retired pins=0 waiters=0 dispatch=quiescent\n'
            raw += f'[R2C] terminal round={round_id} task=0 generation={round_id+1} owner={owner} auditor=0 pin=held state=Finished stack=retained queued=0 late_wake=retired duplicate_exit=rejected path={path}\n'
            if round_id >= 4:
                raw += f'[R2V] cycle round={round_id} source=0 execution={owner} rejected=0x0 released=0x0 timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped\n'
                if cpus > 1:
                    raw += f'[R2C] ordering round={round_id} owner=0 waiter=1 task=0 generation={round_id+1} boundary={round_id-3} recheck=released execution=1 timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped claimed=0x0\n'
        return raw

    def test_matrix(self):
        for cpus in [1, 2, 4, 12]:
            self.assertEqual(validate_native(self.trace(cpus), cpus)['status'], 'pass')

    def test_r2v_alone_is_insufficient(self):
        self.assertEqual(validate_native(NativeEvidenceTests().trace(), 2)['status'], 'fail')

    def test_missing_boundary_or_lifecycle_is_failure(self):
        for prefix in ['[R2C] ordering round=4', '[R2C] ordering round=5', '[R2C] terminal round=3', '[R2C] reuse round=2']:
            raw = '\n'.join(line for line in self.trace().splitlines() if not line.startswith(prefix))
            self.assertEqual(validate_native(raw, 2)['status'], 'fail')

    def test_false_progress_and_identity_fail(self):
        for old, new in [('recheck=released', 'recheck=held'), ('stack=retained', 'stack=freed'),
                         ('generation=6', 'generation=5'), ('path=terminate_current_user_process', 'path=process_exit'),
                         ('boundary=2', 'boundary=1'), ('dispatch=quiescent', 'dispatch=unknown'),
                         ('metadata_slots=1', 'metadata_slots=2'), ('timer_assistance=0', 'timer_assistance=1')]:
            self.assertEqual(validate_native(self.trace().replace(old, new), 2)['status'], 'fail')


if __name__ == '__main__':
    unittest.main()
