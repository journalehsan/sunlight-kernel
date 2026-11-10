"""Destructive controls use retained native traces, never synthetic native proof."""
import os
from pathlib import Path
import unittest
from fastlane_m53r55_assess import assess
from fastlane_m53r53_assess import assess as assess_r53


@unittest.skipUnless(os.environ.get('R55_SERIAL'), 'set R55_SERIAL and R55_ORDER')
class RestrictedLifetimeControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = Path(os.environ['R55_SERIAL']).read_text()
        cls.order = os.environ['R55_ORDER']
        cls.baseline = assess(cls.raw, cls.order)
        if cls.baseline['status'] != 'PASS':
            raise AssertionError(cls.baseline['errors'])

    def test_original_contract_is_preserved(self):
        self.assertEqual(self.baseline['original_scenario_status'], 'UNQUALIFIED')

    def test_missing_boundaries_fail_closed(self):
        for marker in ('[FL-BIND] op=3 ', '[FL-MAP] ', '[FL-TLB] ',
                       'event=frame-freed ', '[R5-QUOTA] event=consumed ',
                       '[R51-SESSION] event=retired ', '[R55] event=retired-denied ',
                       '[R55-SESSION] event=retired-stale-generation-denied',
                       '[FL-CENSUS] checkpoint=550 ', '[FL-CENSUS] checkpoint=540 ',
                       '[R5-PROCESS] event=reaped '):
            with self.subTest(marker=marker):
                raw = '\n'.join(x for x in self.raw.splitlines() if marker not in x)
                self.assertEqual(assess(raw, self.order)['status'], 'FAIL')

    def test_duplicate_refund(self):
        refund = self.baseline['lifetimes'][0]['quota'][1]
        line = self.raw.splitlines()[refund['_line']]
        self.assertEqual(assess(self.raw + '\n' + line, self.order)['status'], 'FAIL')

    def test_changed_payer(self):
        refund = self.baseline['lifetimes'][0]['quota'][1]
        lines = self.raw.splitlines()
        lines[refund['_line']] = lines[refund['_line']].replace('pid=' + refund['pid'], 'pid=999999')
        self.assertEqual(assess('\n'.join(lines), self.order)['status'], 'FAIL')

    def test_premature_refund(self):
        refund = self.baseline['lifetimes'][0]['quota'][1]
        backing = self.baseline['lifetimes'][0]['backing']
        lines = self.raw.splitlines()
        line = lines.pop(refund['_line'])
        lines.insert(backing['_line'], line)
        self.assertEqual(assess('\n'.join(lines), self.order)['status'], 'FAIL')

    def test_opposite_order_rejected(self):
        other = 'cleanup-first' if self.order == 'retirement-first' else 'retirement-first'
        self.assertEqual(assess(self.raw, other)['status'], 'FAIL')

    def test_r53_topology_is_explicit(self):
        strict = assess_r53(self.raw)
        if self.order == 'retirement-first':
            self.assertNotEqual(strict['status'], 'PASS')
            self.assertEqual(assess_r53(self.raw, restricted_extra=True)['status'], 'PASS')
        else:
            self.assertEqual(strict['status'], 'PASS')
            self.assertNotEqual(assess_r53(self.raw, restricted_extra=True)['status'], 'PASS')


if __name__ == '__main__':
    unittest.main()
