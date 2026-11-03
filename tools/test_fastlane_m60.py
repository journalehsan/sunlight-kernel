"""Qualification-boundary controls; models never stand in for native results."""
import copy
import unittest
from fastlane_m60_receipt import qualify, assess
class Qualification(unittest.TestCase):
    def setUp(self):
        self.t={'a':'100','b':'200','ca':'0','cb':'0','ea':'2','eb':'2','ppm':'600','ns':'100','why':'accepted'}
        self.c={'ha':'2500000000','hb':'2500000000','pa':'600','pb':'600','fa':'36','fb':'36','sa':'99','sb':'99'}
    def test_local_pair(self): self.assertEqual(qualify(self.t,self.c),100)
    def test_invalid_pairs(self):
        for field,value,reason in [('cb','1','nonresident'),('eb','3','stale-epoch'),('b','99','backward'),('ea','0','uncalibrated'),('ns','101','guest-host-disagreement')]:
            with self.subTest(field=field):
                t=copy.copy(self.t);t[field]=value;self.assertEqual(qualify(t,self.c),reason)
    def test_invalid_calibration(self):
        for field,value,reason in [('hb','1','calibration-change'),('fa','4','calibration-change'),('pa','0','calibration-change')]:
            with self.subTest(field=field):
                c=copy.copy(self.c);c[field]=value;self.assertEqual(qualify(self.t,c),reason)
        self.c.update(fa='4',fb='4');self.assertEqual(qualify(self.t,self.c),'unstable')
    def test_missing_metadata(self): self.assertEqual(qualify(self.t,{}),'malformed')
    def test_incomplete_native(self): self.assertEqual(assess('',2)['status'],'FAIL')
if __name__=='__main__': unittest.main()
