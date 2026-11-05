"""Native-receipt rejection controls; optional retained guest input is explicit."""
import os
from pathlib import Path
import re
import unittest
from fastlane_m53_receipt import validate
from fastlane_m52_assess import assess
IDENTITY=dict(source=True,elf=True,iso=True,services=True)
class Missing(unittest.TestCase):
    def test_missing(self):self.assertTrue(validate('',2))
    def test_no_synthetic_acceptance(self):self.assertEqual(assess('',2,IDENTITY,measurement=True)['status'],'FAIL')
@unittest.skipUnless(os.environ.get('M53_SERIAL'),'M53_SERIAL required for retained guest controls')
class Native(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw=Path(os.environ['M53_SERIAL']).read_text();cls.cpus=int(os.environ.get('M53_CPUS','2'))
    def rejected(self,raw):self.assertEqual(assess(raw,self.cpus,IDENTITY,measurement=True)['status'],'FAIL')
    def test_original(self):self.assertEqual(assess(self.raw,self.cpus,IDENTITY,measurement=True)['errors'],[])
    def test_omissions(self):
        for prefix in ('[FL-M53-ACTOR]','[FL-M53-SETUP]','[FL-M53-CLOCK]','[FL-M53-KACTOR]','[FL-M53-SAMPLES]','[FL-PF]','[FL-TLB]'):
            self.rejected('\n'.join(l for l in self.raw.splitlines() if not l.startswith(prefix)))
    def test_duplicates(self):
        for prefix in ('[FL-M53-ACTOR]','[FL-M53-KACTOR]','[FL-M53-CLOCK]'):
            self.rejected(self.raw+'\n'+next(l for l in self.raw.splitlines() if l.startswith(prefix)))
    def test_corruption(self):
        for field in ('count','bytes','full','overflow','order','start_sequence','coherent','authentic','next','pid','dropped','end_ms'):
            lines=self.raw.splitlines();i=next(i for i,l in enumerate(lines) if l.startswith('[FL-M53-ACTOR]'))
            lines[i]=re.sub(r'\b'+field+r'=\d+',field+'=99999999999999999999',lines[i]);self.rejected('\n'.join(lines))
    def test_kernel_corruption(self):
        for field in ('notifications','overflow','cpu','inc','vm'):
            lines=self.raw.splitlines();i=next(i for i,l in enumerate(lines) if l.startswith('[FL-M53-KACTOR]'))
            lines[i]=re.sub(r'\b'+field+r'=\d+',field+'=99999',lines[i]);self.rejected('\n'.join(lines))
    def test_time_regression(self):
        self.rejected(re.sub(r'(\[FL-M53-CLOCK\].*? regressions=)0',r'\g<1>1',self.raw,count=1))
    def test_unexpected_message_log(self):self.rejected(self.raw+'\n[FL-M52-MSG] event=producer generation=0 message=0')
if __name__=='__main__':unittest.main()
