import unittest
from fastlane_census_assess import assess

class Trace:
    def __init__(self):
        self.events=[]
        self.census=[]
    def event(self,k,*d):
        d=list(d)+[0]*(8-len(d))
        self.events.append('[FL-OWN] seq=%d kind=%d %s' % (len(self.events),k,' '.join(f'x{i}={v}' for i,v in enumerate(d))))
    def checkpoints(self,extra=None,schema=1):
        for cp in (*range(100,106),999):
            self.event(4,4096,6,0,cp)
            self.event(16,4096,1,4096,0,0,4,1,cp)
            self.event(6,4096,1,22,cp)
            n=1
            if extra and cp>=101:
                extra(self,cp)
                n+=1
            self.event(17,cp,schema)
            self.census.append(f'[FL-CENSUS] checkpoint={cp} tables={n} shm=0')
        return self
    def raw(self):
        n=len(self.events)
        return '\n'.join(self.events+self.census+[f'[FL-CENSUS-TRACE-END] reserved={n} emitted={n} incomplete=0 overflow=0'])

def good():
    t=Trace();t.event(1,4096,6,0,6);return t.checkpoints()

class Controls(unittest.TestCase):
    def test_complete_control(self):
        self.assertEqual(assess(good().raw())['status'],'RECONCILED_OBSERVATIONS')
    def bad(self,raw):
        self.assertEqual(assess(raw)['status'],'FAIL')
    def test_missing_dump(self):self.bad('')
    def test_overflow(self):self.bad(good().raw().replace('overflow=0','overflow=1'))
    def test_incomplete(self):self.bad(good().raw().replace('incomplete=0','incomplete=1'))
    def test_missing_sequence(self):self.bad(good().raw().replace('seq=1 kind=','seq=2 kind=',1))
    def test_counts(self):self.bad(good().raw().replace('tables=1','tables=2',1))
    def test_duplicate_field(self):self.bad(good().raw().replace('kind=1','kind=1 kind=1',1))
    def test_missing_claim(self):self.bad(good().raw().replace('x0=4096 x1=6','x0=8192 x1=6',1))
    def test_unaccounted_growth(self):
        t=Trace();t.event(1,4096,6,0,6);t.event(1,8192,6,0,6)
        t.checkpoints(lambda t,cp:t.event(4,8192,6,0,cp));self.bad(t.raw())
    def test_allocation_identity_reuse(self):
        t=Trace();t.event(1,4096,6,0,6)
        for cp in (*range(100,106),999):
            if cp==101:
                t.event(2,4096,6,0);t.event(1,4096,6,0,6)
            t.event(4,4096,6,0,cp);t.event(16,4096,1,4096,0,0,4,1,cp);t.event(6,4096,1,22,cp);t.event(17,cp)
            t.census.append(f'[FL-CENSUS] checkpoint={cp} tables=1 shm=0')
        r=assess(t.raw());self.assertEqual(r['status'],'RECONCILED_OBSERVATIONS')
        self.assertEqual(len(r['checkpoints'][1]['additions']),1)
    def test_missing_remote_ack(self):
        t=good();t.event(13,1,1,4096,8192,1,2,0,3);t.event(15,1,1,4096,8192,1,2,0);self.bad(t.raw())
    def test_remote_ack_order(self):
        t=good();t.event(13,1,1,4096,8192,1,2,0,3);t.event(15,1,1,4096,8192,1,2,0);t.event(14,1,1,4096,8192,1,0,1,1);self.bad(t.raw())
    def test_remote_ack_identity(self):
        t=good();t.event(13,1,1,4096,8192,1,2,0,3);t.event(14,1,2,4096,8192,1,0,1,1);t.event(15,1,1,4096,8192,1,2,0);self.bad(t.raw())
    def test_unmap_without_flush(self):
        t=good();t.event(18,1,1,4096,8192,4096,0,22,1);t.event(18,1,1,4096,8192,4096,1,22,1);self.bad(t.raw())
    def test_unmap_ack_completion_positive(self):
        t=good();t.event(18,1,1,4096,8192,4096,0,22,1)
        t.event(13,1,1,4096,8192,1,2,0,3);t.event(14,1,1,4096,8192,1,0,1,1);t.event(15,1,1,4096,8192,1,2,0)
        t.event(18,1,1,4096,8192,4096,1,22,1)
        self.assertEqual(assess(t.raw())['status'],'RECONCILED_OBSERVATIONS')
    def test_unfinished_request(self):
        t=good();t.event(13,1,1,4096,8192,1,2,0,3);self.bad(t.raw())


class LifecycleControls(unittest.TestCase):
    def trace(self):
        t=Trace();t.event(1,4096,6,0,6);return t
    def finish(self,t):
        return t.checkpoints(schema=2).raw()
    def shm(self):
        t=self.trace();t.event(1,8192,8,22,8);t.event(7,8192,9,22,0,0,0,0,1)
        t.event(8,8192,9,22,1,0,0,0,1);t.event(19,9,22,7,1,12288,4096);return t
    def unmap(self,t):
        t.event(18,9,1,4096,12288,4096,0,22,1);t.event(13,1,1,4096,12288,1,2,0,3)
        t.event(14,1,1,4096,12288,1,0,1,1);t.event(15,1,1,4096,12288,1,2,0)
        t.event(18,9,1,4096,12288,4096,1,22,1)
    def test_other_accounted_boot_backing(self):
        t=self.trace();t.event(1,8192,15,0,15);t.event(7,8192,9,39321,0,0,0,0,1)
        t.event(10,8192,9,39321,0,1,0,0,1);t.event(2,8192,15,0)
        self.assertEqual(assess(self.finish(t))['status'],'RECONCILED_OBSERVATIONS')
    def test_final_backing_release_order(self):
        t=self.shm();self.unmap(t);t.event(9,8192,9,22,0,0,0,0,1);t.event(2,8192,8,22)
        self.assertEqual(assess(self.finish(t))['status'],'RECONCILED_OBSERVATIONS')
    def test_premature_backing_free(self):
        t=self.shm();t.event(2,8192,8,22);self.assertEqual(assess(self.finish(t))['status'],'FAIL')
    def test_release_without_unmap(self):
        t=self.shm();t.event(9,8192,9,22,0,0,0,0,1);t.event(2,8192,8,22)
        self.assertEqual(assess(self.finish(t))['status'],'FAIL')
    def test_retirement_does_not_release_live_views(self):
        t=self.shm();t.event(10,8192,9,22,1,1,0,0,1);t.event(2,8192,8,22)
        self.assertEqual(assess(self.finish(t))['status'],'FAIL')
    def test_table_free_without_condition(self):
        t=self.trace();t.event(1,8192,6,0,6);t.event(5,8192,1,4096,4096,12288,1,16384,7);t.event(2,8192,6,0)
        self.assertEqual(assess(self.finish(t))['status'],'FAIL')
    def test_table_retirement_covering_shootdown(self):
        t=self.trace();t.event(1,8192,6,0,6);t.event(5,8192,1,4096,4096,12288,1,16384,7)
        t.event(13,1,1,4096,12288,1,2,0,3);t.event(14,1,1,4096,12288,1,0,1,1);t.event(15,1,1,4096,12288,1,2,0)
        t.event(20,8192,1,4096,12288,2,1);t.event(2,8192,6,0)
        self.assertEqual(assess(self.finish(t))['status'],'RECONCILED_OBSERVATIONS')
    def test_active_vm_teardown_rejected(self):
        t=self.trace();t.event(1,8192,6,0,6);t.event(20,8192,1,4096,0,3,2);t.event(2,8192,6,0)
        self.assertEqual(assess(self.finish(t))['status'],'FAIL')
    def test_early_table_retirement_rejected(self):
        t=self.trace();t.event(1,8192,6,0,6);t.event(20,8192,1,4096,12288,2,1);t.event(2,8192,6,0)
        self.assertEqual(assess(self.finish(t))['status'],'FAIL')


if __name__ == '__main__':
    unittest.main()
