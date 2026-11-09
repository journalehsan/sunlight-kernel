"""Synthetic rejection controls for the parser, never native evidence."""
import unittest
from fastlane_m53_quota_validate import assess


def trace(cpus=2):
    rows = ['event=start source=source run=nonce cpus=%d scope=kernel-prerequisite' % cpus,
            'event=rollback pid=1 incarnation=2 vm=3 quota_before=0 quota_after=0 free_before=100 free_after=100 partial_frames=1 overflow=denied over_budget=denied objects=0',
            'event=ledger-boundary process_limit=536870912 quota_after=0 scope=reservation-only']
    if cpus == 1:
        rows += ['event=physical-boundary bytes=536870912 frames_allocated=131072 frames_released=131072 quota_live=536870912 quota_after=0 over_budget=denied free_before=200000 free_after=200000',
                 'event=small-aggregate allocations=64 chunk=8388608 bytes=536870912 frames_allocated=131072 frames_released=131072 quota_after=0 over_budget=denied free_before=200000 free_after=200000']
    for cycle in range(16):
        rows.append('event=cycle cycle=%d pid=1 incarnation=2 vm=3 peer_pid=4 peer_incarnation=5 peer_vm=6 quota_before=0 quota_live=8192 quota_owner_unmapped=8192 quota_after=0 frames_allocated=2 frames_released=2 views_retired=2 objects=0 free_before=100 free_after=100' % cycle)
    rows += ['event=remote acknowledgements=%d expected=%d free_before=100 free_after=100 scope=%s' % (cpus-1,cpus-1,'kernel-shm-prerequisite' if cpus>1 else 'no-remote-cpu'),
             'event=complete source=source run=nonce cpus=%d cycles=16 quota_after=0 free_before=100 free_after=100' % cpus]
    remote_raw = ''
    if cpus > 1:
        remote_raw += '[SHM-M53-REMOTE] event=established pid=7 incarnation=8 vm=9 root=4096 frame=8192 targets=%d quota_live=4096\n' % (((1 << cpus)-1) & ~1)
        remote_raw += '[SHM-M53-REMOTE] event=complete pid=7 incarnation=8 vm=9 frame=8192 reused_after_ack=8192 acknowledgements=%d views_retired=1 frames_released=1 quota_after=0 free_before=100 free_after=100\n' % (cpus-1)
    return '\n'.join('[SHM-M53] '+row for row in rows[:-2])+'\n'+remote_raw+'\n'.join('[SHM-M53] '+row for row in rows[-2:])+'\n'


class QuotaAssessorTests(unittest.TestCase):
    def check(self, raw, status='FAIL', cpus=2):
        self.assertEqual(assess(raw, cpus, 'source', 'nonce')['status'], status)

    def test_complete_controls(self):
        for cpus in (1,2,4,12):
            self.check(trace(cpus), 'PASS', cpus)

    def test_empty_and_boot_only(self):
        self.check('')
        self.check('[SunlightOS] boot OK\n')

    def test_truncated(self):
        self.check(trace().split('[SHM-M53] event=complete')[0])

    def test_duplicate(self):
        self.check(trace()+trace())
        self.check(trace().replace('cycle=0 ', 'cycle=0 cycle=0 '))

    def test_stale_source_nonce_and_cpu(self):
        for before, after in [('source=source','source=old'),('run=nonce','run=old'),('cpus=2','cpus=4')]:
            self.check(trace().replace(before,after))

    def test_missing_cycle_and_frame_drift(self):
        self.check('\n'.join(line for line in trace().splitlines() if 'cycle=3 ' not in line))
        self.check(trace().replace('free_after=100','free_after=99',1))

    def test_retained_charge(self):
        self.check(trace().replace('quota_after=0','quota_after=8192'))
        self.check(trace().replace('quota_owner_unmapped=8192','quota_owner_unmapped=0'))

    def test_invalid_identity(self):
        self.check(trace().replace('peer_incarnation=5','peer_incarnation=0'))
        self.check(trace().replace('vm=3','vm=0'))

    def test_remote_ack_and_frame_reuse(self):
        self.check(trace().replace('acknowledgements=1','acknowledgements=0'))
        self.check(trace().replace('reused_after_ack=8192','reused_after_ack=12288'))
        self.check(trace().replace('targets=2','targets=4'))

    def test_missing_remote_receipts(self):
        self.check('\n'.join(line for line in trace().splitlines() if not line.startswith('[SHM-M53-REMOTE]')))

    def test_guest_failure_even_with_terminal_marker(self):
        self.check(trace()+'\nPANIC\n')

if __name__ == '__main__':
    unittest.main()
