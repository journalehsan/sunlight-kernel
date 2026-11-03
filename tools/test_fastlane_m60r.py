"""M6.0R qualification uses the same clock contract and stricter native receipts."""
import unittest
from test_fastlane_m60 import Qualification
from fastlane_m60r_receipt import assess, CONFIGS

class FairContract(unittest.TestCase):
    def test_common_supported_payload(self):
        self.assertEqual(CONFIGS, ((0,32),(1,32),(2,32)))
    def test_no_old_terminal_acceptance(self):
        self.assertEqual(assess('[FL-M60] case=complete configurations=4 resources=0 integrity=exact historical=UNRESOLVED production=DISABLED',2)['status'],'FAIL')

if __name__=='__main__': unittest.main()
