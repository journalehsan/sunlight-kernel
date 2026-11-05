import unittest

from test_fastlane_f13nr2c import OrderingEvidenceTests
from fastlane_f13nr2d_validate import validate_native


class R2DEvidenceTests(unittest.TestCase):
    def trace(self, cpus=2):
        base = OrderingEvidenceTests().trace(cpus)
        raw = base.replace('rounds=6', 'rounds=8').replace('retired=6', 'retired=8')
        raw = raw.replace(
            f'[R2V] PASS cpus={cpus} rounds=8',
            f'[R2V] PASS cpus={cpus} rounds=8')
        mask = (1 << cpus) - 1 & ~1
        out = []
        # Keep rounds 0..=5 base records; add R2D rounds 6/7.
        for line in raw.splitlines():
            if line.startswith('[R2V] PASS '):
                continue
            out.append(line)
        if cpus > 1:
            for cpu in range(1, cpus):
                out.append(f'[R2V] waiter round=6 cpu={cpu} registered_mask={mask:#x} rejected=1 ack=1 accepted=1 handled=1 pass=1 sends=1 requests=1')
            out.append('[R2D-B] forced round=6 owner=0 waiter=1 task=0 generation=7 boundary=1 registration=published release=claimed busy_deferrals=3 retried=1 request_survives_ack=1 old_dispatch_scoped=cpu cleanup=reaped')
            out.append(f'[R2V] cycle round=6 source=0 execution={cpus - 1} rejected={mask:#x} released={mask:#x} timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped')
            out.append(f'[R2V] waiter round=7 cpu=1 registered_mask=0x2 rejected=1 ack=1 accepted=1 handled=1 pass=1 sends=1 requests=1')
            out.append('[R2D-A] audit round=7 task=0 generation=8 owner=0 waiter=1 pin=held state=Finished stack=retained queued=0 late_wake=retired duplicate_exit=rejected')
            out.append('[R2D-A] forced round=7 owner=0 rejecter=1 task=0 generation=8 rejected=0x2 released=0x2 execution=none pin_release_precedes_reap=1 remote_exit=Finished stack_retained=1 revived=0 duplicate_run=0 waiter_drained=1 pending_dispatch=0 timer_assistance=0')
            out.append('[R2V] cycle round=7 source=0 execution=none rejected=0x2 released=0x2 timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped')
        else:
            out.append('[R2V] cycle round=6 source=0 execution=0 rejected=0x0 released=0x0 timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped')
            out.append('[R2V] cycle round=7 source=0 execution=0 rejected=0x0 released=0x0 timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped')
        out.append(f'[R2V] PASS cpus={cpus} rounds=8 forced={4 * (cpus - 1)} frames_before=100 frames_after=100 metadata_slots=1 retired=8 timer_assistance=0')
        return '\n'.join(out) + '\n'

    def test_complete_matrix(self):
        for cpus in [1, 2, 4, 12]:
            self.assertEqual(validate_native(self.trace(cpus), cpus)['status'], 'pass',
                             msg=f'cpus={cpus}')

    def test_r2c_base_alone_is_insufficient(self):
        self.assertEqual(validate_native(OrderingEvidenceTests().trace(), 2)['status'], 'fail')

    def test_missing_r2d_round_is_failure(self):
        for prefix in ['[R2D-B] forced round=6', '[R2D-A] forced round=7']:
            raw = '\n'.join(line for line in self.trace().splitlines()
                            if not line.startswith(prefix))
            self.assertEqual(validate_native(raw, 2)['status'], 'fail')

    def test_false_r2d_progress_fails(self):
        for old, new in [
            ('storage=retained', 'storage=x'),
            ('pin_release_precedes_reap=1', 'pin_release_precedes_reap=0'),
            ('remote_exit=Finished', 'remote_exit=Ready'),
            ('revived=0', 'revived=1'),
            ('duplicate_run=0', 'duplicate_run=1'),
            ('waiter_drained=1', 'waiter_drained=0'),
            ('pending_dispatch=0', 'pending_dispatch=1'),
            ('busy_deferrals=3', 'busy_deferrals=0'),
            ('registration=published', 'registration=withdrawn'),
            ('release=claimed', 'release=missing'),
            ('request_survives_ack=1', 'request_survives_ack=0'),
            ('rounds=8', 'rounds=6'),
            ('retired=8', 'retired=7'),
        ]:
            if old in self.trace():
                self.assertEqual(validate_native(self.trace().replace(old, new), 2)['status'],
                                 'fail', msg=old)


if __name__ == '__main__':
    unittest.main()
