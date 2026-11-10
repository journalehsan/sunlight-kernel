"""Destructive controls against a retained real restricted diagnostic trace."""
import os
from pathlib import Path
import unittest
from fastlane_m53r54_assess import assess


@unittest.skipUnless(os.environ.get('R54_SERIAL'), 'set R54_SERIAL to retained native trace')
class RestrictedDiagnosticControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = Path(os.environ['R54_SERIAL']).read_text()
        cls.baseline = assess(cls.raw)
        if cls.baseline['diagnostic_status'] != 'PASS':
            raise AssertionError(cls.baseline['errors'])

    def reject_without(self, marker):
        raw = '\n'.join(line for line in self.raw.splitlines() if marker not in line)
        self.assertEqual(assess(raw)['diagnostic_status'], 'FAIL')

    def test_never_promotes_diagnostic(self):
        self.assertEqual(self.baseline['status'], 'UNQUALIFIED')

    def test_missing_role_bind(self):
        self.reject_without('[FL-BIND] op=3 ')

    def test_missing_mapping(self):
        self.reject_without('[FL-MAP] seq=')

    def test_missing_physical_free(self):
        self.reject_without('event=frame-freed ')

    def test_missing_consumption(self):
        self.reject_without('[R5-QUOTA] event=consumed ')

    def test_missing_tlb(self):
        self.reject_without('[FL-TLB] ')

    def test_missing_registry_removal(self):
        self.reject_without('[R51-SESSION] event=retired ')

    def test_missing_inventory(self):
        self.reject_without('[FL-CENSUS] checkpoint=540 ')

    def test_duplicate_refund(self):
        row = self.baseline['lifetimes'][0]['quota'][1]
        original = self.raw.splitlines()[row['_line']]
        self.assertEqual(assess(self.raw + '\n' + original)['diagnostic_status'], 'FAIL')

    def test_wrong_payer(self):
        row = self.baseline['lifetimes'][0]['quota'][1]
        lines = self.raw.splitlines()
        lines[row['_line']] = lines[row['_line']].replace('pid=' + row['pid'], 'pid=999999')
        self.assertEqual(assess('\n'.join(lines))['diagnostic_status'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
