#!/usr/bin/env python3
"""Negative controls for the M5.3R2 broad IPC gate, not native evidence."""
import unittest
from fastlane_m53r2_assess import ASSERTIONS, SUCCESS, assess_ipc


def receipt():
    return '\n'.join((*SUCCESS,
        '[IPC-R2] event=identity pid=11 inc=21 vm=31 call_inc=21 server=12 '
        'server_inc=22 server_vm=32 session=none channel=none mappings=0 '
        'authority=kernel-fixture scope=boot-method',
        '[IPC-R2] event=assertions ' + ' '.join(k + '=1' for k in ASSERTIONS)))


class Controls(unittest.TestCase):
    def test_complete(self):
        self.assertEqual(assess_ipc(receipt())['status'], 'PASS')

    def test_each_old_marker_is_required(self):
        for marker in SUCCESS:
            with self.subTest(marker=marker):
                self.assertEqual(assess_ipc(receipt().replace(marker, ''))['status'], 'FAIL')

    def test_original_unexpected_is_not_overridden_by_success(self):
        for marker in SUCCESS:
            raw = receipt() + '\n' + marker.rsplit(':', 1)[0] + ': UNEXPECTED'
            self.assertEqual(assess_ipc(raw)['status'], 'FAIL')

    def test_other_unexpected_is_not_hidden(self):
        self.assertEqual(assess_ipc(receipt() + '\n[SIG] UNEXPECTED')['status'], 'FAIL')

    def test_each_subassertion_is_required(self):
        for key in ASSERTIONS:
            self.assertEqual(assess_ipc(receipt().replace(key + '=1', key + '=0'))['status'], 'FAIL')

    def test_zero_and_foreign_call_incarnation(self):
        for value in ('0', '22'):
            self.assertEqual(assess_ipc(receipt().replace('call_inc=21', 'call_inc=' + value))['status'], 'FAIL')

    def test_duplicate_details(self):
        self.assertEqual(assess_ipc(receipt() + '\n' + receipt().splitlines()[-1])['status'], 'FAIL')

    def test_duplicate_field(self):
        self.assertEqual(assess_ipc(receipt() + ' stress=1')['status'], 'FAIL')

    def test_missing_details(self):
        self.assertEqual(assess_ipc('\n'.join(SUCCESS))['status'], 'FAIL')

    def test_old_source_scope_has_no_new_identity_claim(self):
        result = assess_ipc('\n'.join(SUCCESS), require_details=False)
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['detail'], {})
        self.assertEqual(result['scheduled_lifecycle'], 'NOT_RUN')

    def test_scope_and_actor_alias_rejected(self):
        for a, b in [('scope=boot-method', 'scope=scheduled'), ('server=12', 'server=11')]:
            self.assertEqual(assess_ipc(receipt().replace(a, b))['status'], 'FAIL')

    def test_panic(self):
        self.assertEqual(assess_ipc(receipt() + '\nPANIC')['status'], 'FAIL')


if __name__ == '__main__':
    unittest.main()
