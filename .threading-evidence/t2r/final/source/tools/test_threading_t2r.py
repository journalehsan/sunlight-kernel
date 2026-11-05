"""Synthetic receipt parser controls; these are never native race evidence."""
import unittest
from threading_t2r_receipt import assess_pilot

LOG = '''[T2R] baseline heap=2051968 workers=0 stacks=4 fd_refs=12 cap_entries=4 reserved=0 pending=0
[T2R] requested-winner actor=0
[T2R] begin case=2 cpus=2 bounded_entry_spins=2000000
[T2R] registered actor=0 pid=10 incarnation=1 vm=AddressSpaceIdentity { pml4_phys: 4096, generation: 1 }
[T2R] commit actor=0 parent=10 tid=20 incarnation=5 slot=4 vm=AddressSpaceIdentity { pml4_phys: 4096, generation: 1 } stack=true queued=1 workers=1
[T2R] outcome case=2 successes=1 commits=1 rejects=0 entry_mask=0
[T2R] child-exit tid=20 incarnation=5 cpu=0 stack_live=true
[T2R] retired tid=20 incarnation=5 slot=4 state=Reaped stack=none pinned=false progress=18
[T2R] heap baseline=2051968 cached_fd_boxes=1 cached_fd_bytes=10240 reaped_stacks=1 drift=0
[T2R] resources point=drained workers=0 occupied=4 slots=5 stacks=4 fd_refs=12 cap_entries=4 heap=2062208 reserved=0 pending=0
[T2R] vm owner=10 incarnation=1 identity=AddressSpaceIdentity { pml4_phys: 4096, generation: 1 } borrowers=0
[T2R] vm owner=11 incarnation=2 identity=AddressSpaceIdentity { pml4_phys: 8192, generation: 2 } borrowers=0
[T2R] vm owner=12 incarnation=3 identity=AddressSpaceIdentity { pml4_phys: 12288, generation: 3 } borrowers=0
[T2R] vm owner=13 incarnation=4 identity=AddressSpaceIdentity { pml4_phys: 16384, generation: 4 } borrowers=0
[T2R] PILOT PASS case=2 cpus=2
'''


class ReceiptTests(unittest.TestCase):
    def test_valid_synthetic(self):
        self.assertTrue(assess_pilot(LOG, 2, 2))

    def test_every_receipt_required_and_unique(self):
        for line in LOG.splitlines(True):
            with self.subTest(line=line):
                self.assertFalse(assess_pilot(LOG.replace(line, ''), 2, 2))
                self.assertFalse(assess_pilot(LOG + line, 2, 2))

    def test_identity_and_resource_negatives(self):
        for old, new in [('retired tid=20', 'retired tid=21'),
                         ('retired tid=20 incarnation=5', 'retired tid=20 incarnation=6'),
                         ('slot=4 state=Reaped', 'slot=3 state=Reaped'),
                         ('fd_refs=12', 'fd_refs=13'), ('cap_entries=4', 'cap_entries=5'),
                         ('stacks=4', 'stacks=5'), ('reserved=0', 'reserved=1'),
                         ('pending=0', 'pending=1'), ('workers=0', 'workers=1'),
                         ('queued=1', 'queued=2'), ('progress=18', 'progress=0'),
                         ('child-exit tid=20', 'child-exit tid=21')]:
            with self.subTest(old=old):
                self.assertFalse(assess_pilot(LOG.replace(old, new), 2, 2))

    def test_wrong_cpu_case_and_panic(self):
        self.assertFalse(assess_pilot(LOG, 2, 4))
        self.assertFalse(assess_pilot(LOG, 1, 2))
        self.assertFalse(assess_pilot(LOG + '[KERNEL PANIC]\n', 2, 2))


if __name__ == '__main__':
    unittest.main()
