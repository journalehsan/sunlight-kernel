"""Mutation controls over a real retained R5 serial; never runtime proofs."""
import os
from pathlib import Path
import unittest
from fastlane_m53r5_assess import assess


class RetainedLifecycleControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = os.environ.get('SUNLIGHT_R5_SERIAL')
        if not path:
            raise unittest.SkipTest('SUNLIGHT_R5_SERIAL must name an actual R5 serial')
        cls.raw = Path(path).read_text()
        if assess(cls.raw)['status'] != 'PASS':
            raise ValueError('retained serial does not pass baseline assessment')

    def reject(self, raw):
        self.assertEqual(assess(raw)['status'], 'FAIL')

    def test_original(self):
        self.assertEqual(assess(self.raw)['status'], 'PASS')

    def test_boot_or_truncated(self):
        self.reject('[SunlightOS] boot OK\n')
        self.reject(self.raw.split('[R5-SNAPSHOT] label=quiescent')[0])

    def test_missing_physical_free(self):
        self.reject('\n'.join(l for l in self.raw.splitlines() if 'event=frame-freed' not in l))

    def test_duplicate_receipt_consumption(self):
        row = next(l for l in self.raw.splitlines() if l.startswith('[R5-QUOTA] event=consumed'))
        self.reject(self.raw.replace(row, row+'\n'+row,1))

    def test_wrong_backing_identity(self):
        row = next(l for l in self.raw.splitlines() if l.startswith('[R5-RESOURCE] event=frame-freed'))
        import re
        self.reject(self.raw.replace(row, re.sub(r'value=\d+', 'value=0', row), 1))

    def test_consumption_before_pmm_free(self):
        lines = self.raw.splitlines()
        a = next(i for i,l in enumerate(lines) if l.startswith('[R5-RESOURCE] event=frame-freed'))
        b = next(i for i,l in enumerate(lines) if l.startswith('[R5-QUOTA] event=consumed'))
        lines[a], lines[b] = lines[b], lines[a]
        self.reject('\n'.join(lines))

    def test_missing_hardware_fault(self):
        self.reject('\n'.join(l for l in self.raw.splitlines() if not l.startswith('[FAULT] #6 ')))

    def test_early_refund(self):
        row = next(l for l in self.raw.splitlines() if l.startswith('[R5-SNAPSHOT] label=survivor-after-reap'))
        self.reject(self.raw.replace(row, row.replace('session_bytes=8192','session_bytes=4096'),1))

    def test_unrelated_tlb_completion(self):
        import re
        row = next(l for l in self.raw.split('[R5-VIEW] event=before-unmap',1)[1].splitlines() if l.startswith('[R5-TLB] event=complete'))
        self.reject(self.raw.replace(row,re.sub(r'request=\d+', 'request=0', row),1))

    def test_stale_provenance(self):
        self.assertEqual(assess(self.raw, source='0'*64)['status'], 'FAIL')
        self.assertEqual(assess(self.raw, run='0'*32)['status'], 'FAIL')
        self.assertEqual(assess(self.raw, cpus=999)['status'], 'FAIL')

    def test_missing_reaper(self):
        self.reject('\n'.join(l for l in self.raw.splitlines() if not l.startswith('[R5-PROCESS] event=reaped')))

    def test_duplicate_fields(self):
        self.reject(self.raw.replace('[R5-QUOTA] event=reserved ', '[R5-QUOTA] event=reserved event=reserved ',1))

if __name__ == '__main__':
    unittest.main()
