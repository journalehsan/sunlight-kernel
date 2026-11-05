"""Focused E3T parser counterexamples; generated strings are never native proof."""
import re
import unittest

from test_fastlane_f11b0e3 import assess_native, event, lifecycle_sample, IDENTITY
from fastlane_f11b0e3_validate import audit_attempt_inventory


def prevention_sample():
    lines = lifecycle_sample('N4').splitlines()
    index = next(i for i, line in enumerate(lines) if 'checkpoint=service-retirement' in line)
    lines[index:index] = [
        event(1, 'n4-precommit'),
        event(2, 'request-binding', recipient=39, recipient_inc=51, recipient_vm=87,
              binding_vm=87, issuer=18, issuer_inc=30, issuer_vm=47, request_generation=4),
        event(3, 'n4-manager-signal', issuer=18, issuer_inc=30, issuer_vm=47,
              manager=10, manager_inc=22, manager_vm=31, signal=15, cpu=1),
        event(4, 'n4-stop-pending', result=15),
        event(5, 'n4-execution-held', owning_core=0, exiting=0),
        event(6, 'mint-committed'),
        '[SESSION-E3] checkpoint=inventory sessions=1 receipts=1 account_grants=1 pending_account_deliveries=1 cpu=0',
    ]
    lines.append('[SESSION-E3-CLIENT] case=N4 checkpoint=broker-pending-observed pid=40 uid=0 result=0')
    sequence = 0
    for i, line in enumerate(lines):
        if ' seq=' in line:
            sequence += 1
            lines[i] = re.sub(r'seq=\d+', f'seq={sequence}', line)
    return '\n'.join(lines)


class RetirementPreventionTests(unittest.TestCase):
    def test_failed_n3m_must_remain_in_attempt_inventory(self):
        failed = dict(serial='original-N3M.serial.log', case='N3M', cpus=2,
                      failure={'type':'RuntimeError'}, serial_sha256='original-digest',
                      validator={'classification':'FORCED_AND_FAILED'})
        self.assertEqual(audit_attempt_inventory([failed], [failed]), [])
        self.assertTrue(audit_attempt_inventory([failed], []))
        self.assertTrue(audit_attempt_inventory([failed], [{**failed, 'validator':{'classification':'FORCED_AND_PASSED'}}]))

    def test_prevention_is_distinct_from_forced_ordering(self):
        result = assess_native(prevention_sample(), 'N4', 2)
        self.assertEqual(result['classification'], 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT', result)
        self.assertIs(result['requested_retirement_before_commit_forced'], False)
        self.assertEqual(assess_native(lifecycle_sample('N4'), 'N4', 2)['classification'], 'NOT_FORCED')

    def test_missing_authentication_signal_ownership_and_cancellation_fail(self):
        raw = prevention_sample()
        for field in ('account-activated', 'login-proof-committed', 'n4-precommit',
                      'n4-manager-signal', 'n4-stop-pending', 'n4-execution-held',
                      'service-retirement', 'peer-closed', 'service-ipc-retired'):
            with self.subTest(field=field):
                result = assess_native(raw.replace('checkpoint='+field+' ', 'checkpoint=absent '), 'N4', 2)
                self.assertNotIn(result['classification'], ('FORCED_AND_PASSED', 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT'))
        for bad in (raw.replace('issuer_inc=30', 'issuer_inc=99'),
                    raw.replace('recipient_vm=87', 'recipient_vm=88'),
                    raw.replace('exiting=0', 'exiting=1'),
                    raw.replace('owning_core=0', 'owning_core=1'),
                    raw.replace('request_generation=4', 'request_generation=0'),
                    raw.replace('epoch=1', 'epoch=0'),
                    raw.replace('pending_call=none', 'pending_call=retained')):
            self.assertNotEqual(assess_native(bad, 'N4', 2)['classification'], 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT')

    def test_retirement_before_commit_cannot_certify_prevention(self):
        raw = prevention_sample()
        bad = raw.replace('checkpoint=mint-committed', 'checkpoint=swap').replace(
            'checkpoint=service-retirement', 'checkpoint=mint-committed').replace(
            'checkpoint=swap', 'checkpoint=service-retirement')
        self.assertNotEqual(assess_native(bad, 'N4', 2)['classification'], 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT')
        lines = raw.splitlines()
        pre = next(i for i, s in enumerate(lines) if 'checkpoint=n4-precommit ' in s)
        mint = next(i for i, s in enumerate(lines) if 'checkpoint=mint-committed ' in s)
        lines[pre], lines[mint] = lines[mint], lines[pre]
        self.assertNotEqual(assess_native('\n'.join(lines), 'N4', 2)['classification'], 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT')

    def test_terminal_inventory_driver_timeout_and_stale_image_fail(self):
        raw = prevention_sample()
        for bad in ('\n'.join(s for s in raw.splitlines() if 'checkpoint=inventory' not in s),
                    '\n'.join(s for i,s in enumerate(raw.splitlines()) if i != max(j for j,t in enumerate(raw.splitlines()) if 'checkpoint=inventory ' in t)),
                    raw+'\n'+event(99, 'mint-committed'), raw+'\n[SESSION-E3-CLIENT] checkpoint=FAIL'):
            self.assertNotEqual(assess_native(bad, 'N4', 2)['classification'], 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT')
        for key in IDENTITY:
            self.assertEqual(assess_native(raw, 'N4', 2, identity={**IDENTITY, key: False})['classification'], 'FORCED_AND_FAILED')
        self.assertEqual(assess_native(raw, 'N4', 2, failure={'type':'TimeoutError'})['classification'], 'FORCED_AND_FAILED')


if __name__ == '__main__':
    unittest.main()
