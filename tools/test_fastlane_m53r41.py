import unittest
from fastlane_m53r41_validate import assess


def good():
    lines = ['[SL41-USER] stage=' + stage for stage in ('entry', 'foreign-authority-denied', 'thread', 'child', 'exec', 'exec-live-thread-denied', 'process-exit-live-thread-reaped', 'suspended-creation-denied', 'resumed', 'dropped-credentials-child', 'dropped-credentials-parent', 'linux-clone-complete', 'child-image-rollback', 'complete')]
    lines.append('[SL41] event=prepublication pid=1 incarnation=10 vm=20 session=7 epoch=8')
    for pid, incarnation, vm, owner, owner_incarnation in [(1,10,20,1,10), (2,11,20,1,10), (3,12,21,3,12), (3,12,22,3,12), (4,13,21,3,12), (5,14,23,5,14), (6,15,23,5,14), (7,16,24,7,16), (8,17,25,8,17)]:
        lines.append(f'[SL41] event=userspace pid={pid} incarnation={incarnation} vm={vm} session=7 epoch=8 owner={owner} owner_incarnation={owner_incarnation} live=1')
    lines += [f'[SL41] event=rollback stage={i} free_before=100 free_after=100 published=0 runnable=0' for i in range(1,5)]
    lines.append('[SL41] event=child-image-rollback free_before=100 free_after=100')
    lines += ['THREAD_PROBE PASS', '[SL41] event=linux-thread pid=10 incarnation=19 vm=26 session=7 epoch=8 owner=9 owner_incarnation=18']
    lines.append('[SL41] event=prepublication pid=8 incarnation=17 vm=25 session=7 epoch=8')
    lines += ['[SL41] event=' + event for event in ('stale-generation-denied', 'bootstrap-replay-denied', 'retirement-fenced')]
    return '\n'.join(lines)


class Controls(unittest.TestCase):
    def test_complete_receipts(self):
        self.assertEqual(assess(good())['status'], 'PASS')

    def test_missing_duplicate_stale_and_failed_receipts(self):
        raw = good()
        mutations = [raw.replace('stage=child', 'stage=absent'), raw.replace('epoch=8 owner=3', 'epoch=9 owner=3'),
                     raw.replace('vm=22', 'vm=21'), raw.replace('owner_incarnation=10 live=1', 'owner_incarnation=99 live=1'),
                     raw.replace('free_after=100', 'free_after=99'), raw.replace('runnable=0', 'runnable=1'),
                     raw + '\n[SL41] event=rollback stage=4 free_before=100 free_after=100 published=0 runnable=0',
                     raw + '\n[SESSION-E3-CLIENT] checkpoint=FAIL line=9', '']
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.assertEqual(assess(mutation)['status'], 'FAIL')
