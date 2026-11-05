"""Identity and fail-closed boundary controls; synthetic, never native proof."""
import unittest
from fastlane_mapping_assess import assess_mapping
from fastlane_scoped_census import assess_trace, fixture_trace
from test_fastlane_mapping import object_trace


class ScopedControls(unittest.TestCase):
    def errors(self, raw):
        return assess_mapping(raw, 2)['errors']

    def test_foreign_free_cannot_replace_fixture_free(self):
        raw = object_trace().replace('event=free generation=1', 'event=free generation=2')
        self.assertIn('missing/duplicate final physical free', self.errors(raw))
        self.assertIn('unattributed physical free', self.errors(raw))

    def test_duplicate_backing_lifetime_rejected(self):
        self.assertIn('duplicate backing lifetime', self.errors(object_trace() +
            '\n[FL-FRAME] seq=11 event=backing generation=1 region=0 frame=16384'))

    def test_duplicate_authenticated_binding_rejected(self):
        raw = object_trace()
        self.assertIn('ambiguous generation binding', self.errors(raw + '\n' + raw.splitlines()[0]))

    def test_quiescence_rejects_outstanding_lifetime(self):
        raw = object_trace().replace('event=free', 'event=reuse')
        raw += '\n[FL-CENSUS] checkpoint=999 objects=0 views=0 frames=0 shm=5 tables=3 free=9'
        self.assertIn('fixture lifetime outstanding at census', self.errors(raw))

    def test_global_variation_is_diagnostic(self):
        raw = '\n'.join(f'[FL-CENSUS] checkpoint={i} objects=0 views=0 frames=0 shm={i} tables={i+2} free={i+4}' for i in range(100,106))
        receipt = assess_mapping(raw,2)
        self.assertNotIn('repeated cycles changed shared frame total', receipt['errors'])
        self.assertEqual(receipt['global_census']['classification'], 'CONCURRENT_DIAGNOSTIC')
        self.assertEqual(len(receipt['global_census']['observations']),6)
        self.assertEqual(receipt['status'],'FAIL')  # counts alone never qualify

    def test_missing_trace_when_required(self):
        self.assertEqual(assess_trace('',required=True)['status'],'NOT_QUALIFIED')
        self.assertEqual(assess_trace('')['status'],'NOT_REQUESTED')

    def test_partial_optional_trace_fails_closed(self):
        for raw in ('[FL-OWN] seq=0 kind=1 x0=1',
                    '[FL-CENSUS-TRACE-END] reserved=1 emitted=1 incomplete=0 overflow=1'):
            self.assertEqual(assess_trace(raw)['status'],'NOT_QUALIFIED')


def ownership_fixture():
    events=[(7,[4096,5,10,0,0,0,0,1]),(19,[5,10,110,210,8192,4096,0,0]),
            (18,[5,210,4096,8192,4096,0,10,1]),(13,[50,210,4096,8192,1,2,0,2]),
            (14,[50,210,4096,8192,1,0,1,1]),(15,[50,210,4096,8192,1,2,0,0]),
            (18,[5,210,4096,8192,4096,1,10,1]),(9,[4096,5,10,0,1,1,0,1]),
            (2,[4096,8,10,0,0,0,0,0])]
    return '\n'.join(f'[FL-OWN] seq={i} kind={kind} '+ ' '.join(f'x{k}={v}' for k,v in enumerate(values))
                     for i,(kind,values) in enumerate(events))


class OwnershipCorrelationControls(unittest.TestCase):
    def receipt(self,raw):
        return fixture_trace(raw,assess_mapping(object_trace(),2)['objects'])

    def test_independent_ledgers_agree(self):
        self.assertEqual(self.receipt(ownership_fixture())['status'],'PASS')

    def test_contradictory_owner(self):
        raw=ownership_fixture().replace('kind=7 x0=4096 x1=5 x2=10','kind=7 x0=4096 x1=5 x2=11')
        self.assertIn('diagnostic fixture backing owner mismatch',self.receipt(raw)['errors'])

    def test_stale_view_generation(self):
        raw=ownership_fixture().replace('kind=19 x0=5 x1=10 x2=110 x3=210','kind=19 x0=5 x1=10 x2=110 x3=212')
        self.assertIn('diagnostic fixture view identity ambiguous/missing',self.receipt(raw)['errors'])

    def test_unrelated_physical_free(self):
        raw=ownership_fixture().replace('kind=2 x0=4096 x1=8 x2=10','kind=2 x0=4096 x1=8 x2=11')
        self.assertIn('diagnostic fixture physical retirement mismatch',self.receipt(raw)['errors'])

    def test_missing_real_target_ack(self):
        raw='\n'.join(l for l in ownership_fixture().splitlines() if 'kind=14 ' not in l)
        self.assertIn('diagnostic fixture remote ACK mismatch/order',self.receipt(raw)['errors'])

    def test_duplicate_pmm_free_before_reuse(self):
        raw=ownership_fixture()+'\n[FL-OWN] seq=9 kind=2 x0=4096 x1=8 x2=10 x3=0 x4=0 x5=0 x6=0 x7=0'
        self.assertIn('diagnostic fixture premature/duplicate physical retirement',self.receipt(raw)['errors'])

    def test_legitimate_new_frame_lifetime(self):
        raw=ownership_fixture()+'\n[FL-OWN] seq=9 kind=1 x0=4096 x1=8 x2=11 x3=0 x4=0 x5=0 x6=0 x7=0'
        raw+='\n[FL-OWN] seq=10 kind=2 x0=4096 x1=8 x2=11 x3=0 x4=0 x5=0 x6=0 x7=0'
        self.assertEqual(self.receipt(raw)['status'],'PASS')


if __name__ == '__main__':
    unittest.main()
