"""Destructive controls use retained real serial; they are not native evidence."""
import os
from pathlib import Path
import unittest

from fastlane_m53r53_assess import assess, records


class SessionCleanupEvidenceControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = os.environ.get('SUNLIGHT_R53_SERIAL')
        if not path:
            raise unittest.SkipTest('set SUNLIGHT_R53_SERIAL to final native serial')
        cls.raw = Path(path).read_text()
        cls.baseline = assess(cls.raw)
        if cls.baseline['status'] != 'PASS':
            raise AssertionError(cls.baseline)

    def reject_without(self, prefix, **fields):
        selected = [r for r in records(self.raw, prefix)
                    if all(r.get(k) == v for k, v in fields.items())]
        self.assertTrue(selected)
        remove = {r['_line'] for r in selected}
        broken = '\n'.join(line for i, line in enumerate(self.raw.splitlines()) if i not in remove)
        self.assertEqual(assess(broken)['status'], 'UNQUALIFIED')

    def test_baseline_and_both_winners(self):
        self.assertEqual(self.baseline['status'], 'PASS')
        self.assertEqual(len(self.baseline['orders']), 2)

    def test_empty(self):
        self.assertEqual(assess('')['status'], 'UNQUALIFIED')

    def test_missing_retirement(self):
        self.reject_without('[R51-SESSION] ', event='retired')

    def test_missing_owner_commit(self):
        self.reject_without('[R5-PROCESS] ', event='termination-committed',
                            pid=self.baseline['orders'][0]['owner']['pid'])

    def test_missing_reap(self):
        self.reject_without('[R5-PROCESS] ', event='reaped',
                            pid=self.baseline['orders'][1]['owner']['pid'])

    def test_missing_surviving_peer_reap(self):
        self.reject_without('[R5-PROCESS] ', event='reaped',
                            pid=self.baseline['survivor_reaped']['pid'])

    def test_missing_reference_commit(self):
        self.reject_without('[R5-RESOURCE] ', event='reference-committed',
                            resource=self.baseline['orders'][0]['resource'])

    def test_missing_physical_free(self):
        self.reject_without('[R5-RESOURCE] ', event='frame-freed',
                            resource=self.baseline['orders'][1]['resource'])

    def test_missing_mapping_retirement(self):
        self.reject_without('[R5-VIEW] ', event='after-unmap',
                            resource=self.baseline['orders'][0]['resource'])

    def test_missing_exact_tlb_ack(self):
        receipt = self.baseline['lifetimes'][1]['views'][0]['tlb_complete']
        self.reject_without('[R5-TLB] ', event='complete', request=receipt['request'])

    def test_missing_ready_actor(self):
        self.reject_without('[R53] ', event='owner-ready', mode='r53-cleanup-first')

    def test_missing_admission_denial(self):
        self.reject_without('[R53] ', event='owner-retired-denied')

    def test_zero_charge_authority_still_absent(self):
        self.reject_without('[R53] ', event='zero-charge-denied')

    def test_duplicate_refund(self):
        resource = self.baseline['orders'][0]['resource']
        row = next(r for r in records(self.raw, '[R5-QUOTA] ')
                   if r['event'] == 'consumed' and r['resource'] == resource)
        self.assertEqual(assess(self.raw + '\n' + self.raw.splitlines()[row['_line']])['status'], 'UNQUALIFIED')

    def test_swapped_actual_winner(self):
        before, after = (o['owner']['pid'] for o in self.baseline['orders'])
        broken = self.raw.replace(f'before_owner={before} after_owner={after}',
                                  f'before_owner={after} after_owner={before}')
        self.assertNotEqual(broken, self.raw)
        self.assertEqual(assess(broken)['status'], 'UNQUALIFIED')

    def test_premature_refund(self):
        lines = self.raw.splitlines()
        row = next(r for r in records(self.raw, '[R5-SNAPSHOT] ')
                   if r['label'] == 'r53-retirement-before-cleanup')
        lines[row['_line']] = lines[row['_line']].replace('session_bytes=12288', 'session_bytes=8192')
        self.assertEqual(assess('\n'.join(lines))['status'], 'UNQUALIFIED')

    def test_stale_payer_incarnation(self):
        row = self.baseline['lifetimes'][1]['charge'][1]
        lines = self.raw.splitlines()
        lines[row['_line']] = lines[row['_line']].replace('incarnation=' + row['incarnation'], 'incarnation=999999')
        self.assertEqual(assess('\n'.join(lines))['status'], 'UNQUALIFIED')

    def test_post_retirement_reservation(self):
        row = self.baseline['lifetimes'][1]['charge'][0]
        self.assertEqual(assess(self.raw + '\n' + self.raw.splitlines()[row['_line']])['status'], 'UNQUALIFIED')


if __name__ == '__main__':
    unittest.main()
