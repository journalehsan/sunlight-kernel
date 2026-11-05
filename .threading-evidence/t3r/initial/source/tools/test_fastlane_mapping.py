"""Synthetic assessor rejection controls only; never stored as native evidence."""
import unittest
from fastlane_mapping_assess import assess_mapping

def object_trace(write=1, ack=True, free_seq=9, target=2):
    out=['[FL-BIND] op=2 generation=1 producer=10 producer_inc=110 producer_vm=210 consumer=11 consumer_inc=111 consumer_vm=211',
         '[FL-FRAME] seq=1 event=backing generation=1 region=0 frame=4096',
         f'[FL-MAP] seq=2 event=map generation=1 region=0 role=0 pid=10 inc=110 vm=210 va=8192 frame=4096 write={write} nx=1 cpu=0',
         '[FL-MAP] seq=3 event=request generation=1 region=0 pid=10 inc=110 vm=210 va=8192 cpu=0',
         f'[FL-TLB] seq=4 event=request generation=1 region=0 vm=210 va=8192 request=50 targets={target} cpu=0 observed=0 flushed=0']
    if ack:out.append(f'[FL-TLB] seq=5 event=ack generation=1 region=0 vm=210 va=8192 request=50 targets={target} cpu=1 observed=50 flushed=1')
    out += [f'[FL-TLB] seq=6 event=complete generation=1 region=0 vm=210 va=8192 request=50 targets={target} cpu=0 observed=0 flushed=0',
            '[FL-MAP] seq=7 event=complete generation=1 region=0 pid=10 inc=110 vm=210 va=8192 cpu=0',
            f'[FL-FRAME] seq={free_seq} event=free generation=1 region=0 frame=4096']
    return '\n'.join(out)

class MappingControls(unittest.TestCase):
    def errors(self,raw):return assess_mapping(raw,2)['errors']
    def test_empty_never_passes(self):self.assertEqual(assess_mapping('',2)['status'],'FAIL')
    def test_valid_object_has_no_order_or_authority_error(self):
        errors=self.errors(object_trace())
        for message in ('unmap/TLB event order','mapping lacks authenticated exact role binding','remote acknowledgements missing/duplicate','frame freed before unmap and ACK completion'):
            self.assertNotIn(message,errors)
    def test_missing_ack(self):self.assertIn('remote acknowledgements missing/duplicate',self.errors(object_trace(ack=False)))
    def test_wrong_ack_sequence(self):self.assertIn('ACK does not match actual handler request',self.errors(object_trace().replace('observed=50','observed=49')))
    def test_early_frame_free(self):self.assertIn('frame freed before unmap and ACK completion',self.errors(object_trace(free_seq=5)))
    def test_duplicate_free(self):self.assertIn('missing/duplicate final physical free',self.errors(object_trace()+'\n[FL-FRAME] seq=10 event=free generation=1 region=0 frame=4096'))
    def test_wrong_frame(self):self.assertIn('wrong backing frame or publication order',self.errors(object_trace().replace('va=8192 frame=4096','va=8192 frame=8192')))
    def test_foreign_vm(self):self.assertIn('mapping lacks authenticated exact role binding',self.errors(object_trace().replace(' vm=210',' vm=212')))
    def test_foreign_process(self):self.assertIn('mapping lacks authenticated exact role binding',self.errors(object_trace().replace('pid=10','pid=12')))
    def test_changed_incarnation(self):self.assertIn('mapping lacks authenticated exact role binding',self.errors(object_trace().replace(' inc=110',' inc=112')))
    def test_consumer_cannot_write_payload(self):
        raw=object_trace().replace('region=0','region=2').replace('role=0 pid=10 inc=110 vm=210','role=1 pid=11 inc=111 vm=211')
        self.assertIn('writable role escalation or executable mapping',self.errors(raw))
    def test_executable_mapping(self):self.assertIn('writable role escalation or executable mapping',self.errors(object_trace().replace('nx=1','nx=0')))
    def test_wrong_tlb_completion(self):self.assertIn('TLB completion changed identity',self.errors(object_trace().replace('seq=6 event=complete generation=1 region=0 vm=210 va=8192 request=50','seq=6 event=complete generation=1 region=0 vm=210 va=8192 request=51')))
    def test_invalid_cpu_mask(self):self.assertIn('invalid remote target mask',self.errors(object_trace(target=8)))
    def test_zero_target_is_not_remote(self):self.assertIn('remote and zero-target cases must be distinguished and exercised',self.errors(object_trace(ack=False,target=0)))
    def test_missing_unmap(self):self.assertIn('mapping lacks exact unmap request/completion',self.errors(object_trace().replace('event=complete generation=1 region=0 pid','event=missing generation=1 region=0 pid')))
    def test_drift(self):self.assertIn('restricted resource drift',self.errors('[FL-CENSUS] checkpoint=100 objects=1 views=0 frames=0 shm=1 tables=1'))
    def test_rollback_leak(self):self.assertIn('rollback did not reconcile real allocator',self.errors('[FL-ROLLBACK] free_before=10 free_after=9 published=0'))
    def test_duplicate_field(self):self.assertIn('duplicate trace field',self.errors('[FL-M2] case=start case=start'))
    def test_unannounced_fault(self):self.assertIn('unexpected or missing page fault',self.errors('[FAULT] #14 pid=10 err=0x7'))
    def test_guest_assertion(self):self.assertIn('unexpected guest assertion',self.errors('PANIC'))

if __name__=='__main__':unittest.main()
