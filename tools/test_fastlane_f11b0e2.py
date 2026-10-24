import unittest
from fastlane_f11b0e2_validate import assess_session

class SessionEvidenceTests(unittest.TestCase):
    def test_empty_boot_cannot_certify_session_protocol(self):
        r = assess_session('[SunlightOS] boot OK', 12)
        self.assertEqual(r['classification'], 'FORCED_AND_FAILED')
        self.assertTrue(all(v == 'NOT_FORCED' for v in r['native_session_cases'].values()))
        self.assertEqual(r['session_authority'], 'PARTIAL')

    def test_isolated_native_boundary_never_certifies_trusted_authentication(self):
        raw = '\n'.join(f'[SESSION-E2] sequential case={i} cpu=0 scope=isolated-production-broker live=0' for i in range(4))
        raw += '\n[SESSION-E2] boundary PASS registrations=4 revoked=2 expired=1 owner_retired=1 live=0 receipts=0 grants=0 pending_commit=0 consumption_refs=0 revocation_ops=0 ipc_completions=0 exhausted=denied old_epoch=denied cpu=0 scope=isolated-production-broker'
        r = assess_session(raw, 2)
        self.assertEqual(r['classification'], 'FORCED_AND_PASSED')
        self.assertEqual(r['trusted_service_authentication'], 'NOT_TESTED')
        self.assertEqual(r['native_session_cases']['N3'], 'NOT_FORCED')
        self.assertEqual(assess_session(raw.replace('grants=0', 'grants=1'), 2)['classification'], 'FORCED_AND_FAILED')
        self.assertEqual(assess_session(raw.replace('cpu=0', 'cpu=2'), 2)['classification'], 'FORCED_AND_FAILED')
