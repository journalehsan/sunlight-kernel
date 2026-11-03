"""Fail-closed parser controls; retained serial replays are not new native guests."""
import os
from pathlib import Path
import re
import unittest
from fastlane_m52_assess import assess
IDENTITY = dict(source=True, elf=True, iso=True, services=True)

class FailClosed(unittest.TestCase):
    def test_missing_native_evidence(self):
        self.assertEqual(assess('', 2, IDENTITY)['status'], 'FAIL')
    def test_no_identity(self):
        self.assertEqual(assess('', 2)['status'], 'FAIL')

@unittest.skipUnless(os.environ.get('M52_SERIAL'), 'set M52_SERIAL to retained native serial for mutation controls')
class NativeReplay(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = Path(os.environ['M52_SERIAL']).read_text()
        cls.cpus = int(os.environ.get('M52_CPUS','2'))
    def check_rejected(self, raw, identity=IDENTITY, failure=None):
        self.assertEqual(assess(raw,self.cpus,identity,failure)['status'],'FAIL')
    def test_original(self):
        self.assertEqual(assess(self.raw,self.cpus,IDENTITY)['errors'],[])
    def test_missing_receipt_classes(self):
        for prefix in ('[FL-M52-MSG]', '[FL-NATIVE-RING]', '[FL-BIND]', '[FL-MAP]', '[FL-TLB]', '[FL-FRAME]', '[FL-BELL]', '[FL-CENSUS]', '[FL-M52-CLEAN]', '[FL-M52-KDIAG]', '[FL-PF]', '[FL-BELL-RACE]'):
            with self.subTest(prefix=prefix):
                self.check_rejected('\n'.join(l for l in self.raw.splitlines() if not l.startswith(prefix)))
    def test_duplicate_message(self):
        line=next(l for l in self.raw.splitlines() if l.startswith('[FL-M52-MSG]'))
        self.check_rejected(self.raw+'\n'+line)
    def test_bad_generation_length_integrity(self):
        for field,value in [('slot_generation','999'),('bytes','999'),('coherent','1'),('authentic','1'),('pid','0')]:
            with self.subTest(field=field):
                lines=self.raw.splitlines()
                i=next(i for i,l in enumerate(lines) if l.startswith('[FL-M52-MSG] event=consumer'))
                lines[i]=re.sub(r'\b'+field+r'=\d+',field+'='+value,lines[i])
                self.check_rejected('\n'.join(lines))
    def test_foreign_activation(self):
        self.check_rejected(re.sub(r'(\[FL-NATIVE-RING\].*? pid=)\d+',r'\g<1>0',self.raw,count=1))
    def test_no_remote_ack(self):
        self.check_rejected('\n'.join(l for l in self.raw.splitlines() if not l.startswith('[FL-TLB]') or 'event=ack' not in l))
    def test_premature_free(self):
        self.check_rejected(re.sub(r'(\[FL-FRAME\] seq=)\d+( event=free)',r'\g<1>0\2',self.raw,count=1))
    def test_mediated_transfer(self):
        self.check_rejected(self.raw+'\n[FL-REPEAT] event=publish generation=1')
    def test_omitted_identity_or_failure(self):
        self.check_rejected(self.raw,dict(IDENTITY,iso=False))
        self.check_rejected(self.raw,failure={'type':'Timeout'})
    def test_unexpected_fault(self):
        self.check_rejected(self.raw+'\n[FAULT] Page Fault at unexpected')

if __name__=='__main__': unittest.main()
