"""Clock controls and retained-native receipt mutations; no fabricated native claims."""
import os
import re
import unittest
from pathlib import Path
from fastlane_m61a_receipt import assess, clock

class Clock(unittest.TestCase):
    def setUp(self):
        self.a=dict(cpu='0',epoch='2',ns='10',hz='2500000000',ppm='800',flags='36',spread='200')
        self.b={**self.a,'ns':'100'}
    def test_valid(self):self.assertEqual(clock(self.a,self.b),90)
    def test_rejects(self):
        for field,value in [('cpu','1'),('epoch','3'),('ns','9'),('hz','123'),('ppm','0'),('flags','0'),('spread','201')]:
            with self.subTest(field=field):self.assertIsInstance(clock(self.a,{**self.b,field:value}),str)
    def test_empty(self):self.assertEqual(assess('',2)['status'],'FAIL')

@unittest.skipUnless(os.environ.get('M61A_SERIAL'),'native receipt supplied explicitly')
class Native(unittest.TestCase):
    def setUp(self):
        self.raw=Path(os.environ['M61A_SERIAL']).read_text()
        self.cpus=int(os.environ.get('M61A_CPUS','2'))
        self.assertEqual(assess(self.raw,self.cpus)['status'],'PASS')
    def mutate(self,prefix,field=None,value=None,remove=False,duplicate=False,predicate=lambda _:True):
        ls=self.raw.splitlines();i=next(i for i,l in enumerate(ls) if l.startswith(prefix) and predicate(l))
        if remove:ls.pop(i)
        elif duplicate:ls.insert(i,ls[i])
        else:
            changed,n=re.subn(r'\b'+field+r'=\S+',field+'='+value,ls[i]);self.assertEqual(n,1);ls[i]=changed
        data='\n'.join(ls)+'\n';result=assess(data,self.cpus)
        self.assertEqual(result['status'],'FAIL',result)
        out=os.environ.get('M61A_MODELS')
        if out:
            name=self.id().split('.')[-1]+'-'+str(field or ('remove' if remove else 'duplicate'))
            Path(out).mkdir(parents=True,exist_ok=True);Path(out,name+'.serial.model.log').write_text(data)
    def test_missing_start(self):self.mutate('[FL-M61A-STAMP] ',remove=True)
    def test_duplicate_stop(self):self.mutate('[FL-M61A-STAMP] ',duplicate=True,predicate=lambda l:'event=stop ' in l)
    def test_wrong_actor(self):self.mutate('[FL-M61A-CPU] ','pid','99999')
    def test_stale_incarnation(self):self.mutate('[FL-M61A-STAMP] ','inc','99999')
    def test_stale_vm(self):self.mutate('[FL-M61A-STAMP] ','vm','99999')
    def test_wrong_generation(self):self.mutate('[FL-M61A-CPU] ','generation','99999')
    def test_wrong_window(self):self.mutate('[FL-M61A-STAMP] ','w','99999')
    def test_counter_exhausted(self):self.mutate('[FL-M61A-CPU] ','overflow','1')
    def test_discontinuity(self):self.mutate('[FL-M61A-CPU] ','discontinuities','1')
    def test_backward_same_cpu(self):self.mutate('[FL-M61A-STAMP] ','ns','1',predicate=lambda l:'event=stop ' in l)
    def test_stale_clock_epoch(self):self.mutate('[FL-M61A-STAMP] ','epoch','99999',predicate=lambda l:'event=stop ' in l)
    def test_changed_calibration(self):self.mutate('[FL-M61A-STAMP] ','hz','99999',predicate=lambda l:'event=stop ' in l)
    def test_excess_uncertainty(self):self.mutate('[FL-M61A-STAMP] ','ppm','99999')
    def test_migration_hidden(self):self.mutate('[FL-M61A-CPU] ','migrated','0',predicate=lambda l:'w=2 ' in l)
    def test_migration_misqualified(self):self.mutate('[FL-M61A-CPU] ','valid','1',predicate=lambda l:'w=2 ' in l)
    def test_missing_exit_abort(self):self.mutate('[FL-M61A-ABORT] ',remove=True)
    def test_wrong_aborted_actor(self):self.mutate('[FL-M61A-ABORT] ','pid','99999')
    def test_wrong_ack(self):self.mutate('[FL-M61A-P] ','replies','143')
    def test_wrong_integrity(self):self.mutate('[FL-M61A-R] ','verified','143')
    def test_missing_free(self):self.mutate('[FL-FRAME] ',remove=True,predicate=lambda l:'event=free ' in l)
    def test_duplicate_free(self):self.mutate('[FL-FRAME] ',duplicate=True,predicate=lambda l:'event=free ' in l)
    def test_premature_free(self):self.mutate('[FL-FRAME] ','seq','0',predicate=lambda l:'event=free ' in l)
    def test_wrong_tlb_request(self):self.mutate('[FL-TLB] ','request','99999',predicate=lambda l:'event=complete ' in l)
    def test_nonzero_inventory(self):self.mutate('[FL-CENSUS] ','frames','1')
    def test_pending_ipc(self):self.mutate('[SESSION-E3] ','pending_call','some',predicate=lambda l:'checkpoint=fixture-ipc-cleaned ' in l)
    def test_missing_kernel_cleanup(self):self.mutate('[SESSION-E3] ',remove=True,predicate=lambda l:'checkpoint=fixture-ipc-cleaned ' in l)
    def test_duplicate_receipt(self):self.mutate('[FL-M61A-CPU] ',duplicate=True)
    def test_missing_negative(self):self.mutate('[FL-M61A-NEG] ',remove=True)
    def test_no_foreign_check(self):self.mutate('[FL-M61A-FOREIGN] ',remove=True)
    def test_changed_warmup(self):self.mutate('[FL-M61A-WARM] ','completed','15')

if __name__=='__main__':unittest.main()
