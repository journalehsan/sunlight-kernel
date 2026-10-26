"""Parser counterexamples only; these synthetic strings are not native evidence."""
import unittest
import re
from fastlane_f11b0e3_validate import assess_native as validate, assess_replacement

IDENTITY = dict(iso_matches=True, elf_matches=True, services_match=True, source_matches=True)


def assess_native(raw, case, cpus, **kwargs):
    return validate(raw, case, cpus, identity=kwargs.pop('identity', IDENTITY), **kwargs)


def event(seq, checkpoint, **fields):
    values = dict(seq=seq, checkpoint=checkpoint, pid=18, incarnation=30, vm=47,
                  account=1, sessiond=0, tty=0, session=1, epoch=1, result=0, cpu=0)
    values.update(fields)
    return '[SESSION-E3] ' + ' '.join(f'{k}={v}' for k, v in values.items())


def sample(case='N1'):
    lines = ['[SESSION-E3] checkpoint=inventory sessions=0 receipts=0 account_grants=0 pending_account_deliveries=0 cpu=0',
             event(1, 'account-activated'), event(2, 'login-proof-committed'),
             event(3, 'registered', pid=29, incarnation=41, vm=69, account=0, sessiond=1)]
    lines += [event(4,'authenticated-requester',pid=39,incarnation=51,vm=87,account=0),
              event(5,'request-binding',recipient=39,recipient_inc=51,recipient_vm=87,binding_vm=87,
                    issuer=18,issuer_inc=30,issuer_vm=47,request_generation=4)]
    for checkpoint, pid in [('request-start',39), ('controller-ready',40),
                            ('policy-accepted-rendezvous',40), ('fixture-complete',39)]:
        lines.append(f'[SESSION-E3-CLIENT] case={case} checkpoint={checkpoint} pid={pid} uid=0 result=0')
    if case == 'N1':
        lines += ['[SESSION-E3-UAC] case=1 stage=0 pid=18 session=1 epoch=1',
                  event(4, 'transition-committed', pid=29, account=0, sessiond=1, result=3, cpu=1),
                  event(5, 'mint-syscall-rejected', result=11),
                  f'[SESSION-E3-CLIENT] case={case} checkpoint=mint-response pid=39 uid=0 result={0xC2FE}']
    else:
        lines += ['[SESSION-E3-UAC] case=3 stage=0 pid=18 session=1 epoch=1',
                  event(4,'boundary-ready',result=1),
                  event(5,'boundary-ready',pid=29,account=0,sessiond=1,result=2,cpu=1),
                  event(6,'mint-committed'),
                  event(7,'transition-committed',pid=29,account=0,sessiond=1,result=3,cpu=1),
                  f'[SESSION-E3-CLIENT] case={case} checkpoint=mint-response pid=39 uid=0 result={0xC2FF}',
                  f'[SESSION-E3-CLIENT] case={case} checkpoint=post-logout-authority-denied pid=39 uid=0 result=0']
    lines += [event(8,'fixture-ipc-cleaned',pid=39,owned_endpoints=0,pending_call='none',reply_target='none',deferred_targets=0,pending_commits=0),
              event(9,'fixture-ipc-cleaned',pid=40,owned_endpoints=0,pending_call='none',reply_target='none',deferred_targets=0,pending_commits=0),
              '[SESSION-E3] checkpoint=inventory sessions=0 receipts=0 account_grants=0 pending_account_deliveries=0 cpu=1']
    sequence = 0
    for index, line in enumerate(lines):
        if ' seq=' in line:
            sequence += 1
            lines[index] = re.sub(r'seq=\d+', f'seq={sequence}', line)
    return '\n'.join(lines)


def lifecycle_sample(case='N5'):
    lines = ['[SESSION-E3] checkpoint=inventory sessions=0 receipts=0 account_grants=0 pending_account_deliveries=0 cpu=0',
             event(1,'account-activated'), event(2,'login-proof-committed'),
             event(3,'registered',pid=29,account=0,sessiond=1),
             event(4,'authenticated-requester',pid=39,incarnation=51,vm=87,account=0),
             event(5,'request-binding',recipient=39,recipient_inc=51,recipient_vm=87,binding_vm=87,
                   issuer=18,issuer_inc=30,issuer_vm=47,request_generation=4)]
    if case != 'N4':
        lines += [event(6,'mint-committed'),
                  '[SESSION-E3] checkpoint=inventory sessions=1 receipts=1 account_grants=1 pending_account_deliveries=1 cpu=0']
    lines += [event(7,'service-retirement',account_pid=18,account_inc=30,account_vm=47,retiring_pid=18,cpu=1),
              event(8,'peer-closed',recipient=39,recipient_inc=51,request_generation=4,cpu=1),
              event(9,'service-ipc-retired',reply_target='none',deferred_targets=0,pending_call='none',cpu=1),
              event(10,'account-activated',pid=42,incarnation=54,vm=93,cpu=1),
              event(11,'mint-syscall-rejected',pid=42,incarnation=54,vm=93,result=11),
              event(12,'fixture-ipc-cleaned',pid=39,owned_endpoints=0,pending_call='none',reply_target='none',deferred_targets=0,pending_commits=0),
              event(13,'fixture-ipc-cleaned',pid=40,owned_endpoints=0,pending_call='none',reply_target='none',deferred_targets=0,pending_commits=0),
              '[SESSION-E3] checkpoint=inventory sessions=0 receipts=0 account_grants=0 pending_account_deliveries=0 cpu=1']
    for checkpoint,pid in [('request-start',39),('controller-ready',40),('policy-accepted-rendezvous',40),
                           ('old-call-peer-closed',39),('replacement-old-session-denied',39),('fixture-complete',39)]:
        lines += [f'[SESSION-E3-CLIENT] case={case} checkpoint={checkpoint} pid={pid} uid=0 result=0']
    ordinal = {'N4':4,'N5':5,'N6':6}[case]
    lines += [f'[SESSION-E3-UAC] case={ordinal} stage={0 if case == "N4" else 1} pid=18 session=1 epoch=1']
    return '\n'.join(lines)


class AuthenticatedEvidenceTests(unittest.TestCase):
    def test_lifecycle_cancellation_requires_actual_old_call_and_retirement(self):
        raw = lifecycle_sample()
        self.assertEqual(assess_native(raw,'N5',2)['classification'],'FORCED_AND_PASSED')
        for bad in (raw.replace('checkpoint=peer-closed','checkpoint=absent'),
                    raw.replace('checkpoint=service-retirement','checkpoint=absent'),
                    raw.replace('checkpoint=account-activated','checkpoint=absent'),
                    raw.replace('pending_account_deliveries=1','pending_account_deliveries=0'),
                    raw.replace('recipient_inc=51 request_generation=4','recipient_inc=99 request_generation=4'),
                    raw.replace('retiring_pid=18','retiring_pid=29'),
                    raw.replace('pending_call=none','pending_call=retained')):
            self.assertNotEqual(assess_native(bad,'N5',2)['classification'],'FORCED_AND_PASSED')

    def test_policy_pause_and_replacement_denial_do_not_certify_n4(self):
        result = assess_native(lifecycle_sample('N4'),'N4',2)
        self.assertEqual(result['classification'],'NOT_FORCED')
        self.assertTrue(any('original Account broker precommit' in m for m in result['missing_checkpoints']))

    def test_restart_without_new_login_does_not_certify_fresh_session(self):
        raw = lifecycle_sample('N6')
        self.assertEqual(assess_native(raw,'N6',2)['classification'],'FORCED_AND_PASSED')
        result = assess_replacement(raw,len(raw),2,identity=IDENTITY)
        self.assertNotEqual(result['classification'],'FORCED_AND_PASSED')
        self.assertFalse(result['nonaliasing'])

    def test_service_pid_reuse_is_observed_separately_from_recipient_gate(self):
        raw = lifecycle_sample().replace('pid=42 incarnation=54','pid=18 incarnation=54')
        result = assess_native(raw,'N5',2)
        self.assertEqual(result['classification'],'FORCED_AND_PASSED')
        self.assertTrue(result['service_numeric_pid_reused'])
        self.assertEqual(result['actual_pid_reuse'],'NOT_FORCED')

    def test_boot_and_isolated_witness_do_not_certify(self):
        for raw in ('[SunlightOS] boot PASS', '[SESSION-E2] boundary PASS'):
            self.assertEqual(assess_native(raw,'N1',12)['classification'],'NOT_FORCED')

    def test_bounded_fixture_example_and_missing_boundary(self):
        raw = sample()
        self.assertEqual(assess_native(raw,'N1',2)['classification'],'FORCED_AND_PASSED')
        raw = '\n'.join(line for line in raw.splitlines() if 'checkpoint=transition-committed' not in line)
        self.assertEqual(assess_native(raw,'N1',2)['classification'],'NOT_FORCED')

    def test_untrusted_issuer_wrong_epoch_and_leaked_reference_fail(self):
        raw = sample()
        for bad in (raw.replace('account=1','account=0'), raw.replace('checkpoint=mint-syscall-rejected','epoch=2 checkpoint=mint-syscall-rejected'),
                    raw.replace('owned_endpoints=0','owned_endpoints=1'), raw.replace('receipts=0','receipts=1')):
            self.assertNotEqual(assess_native(bad,'N1',2)['classification'],'FORCED_AND_PASSED')

    def test_cross_cpu_admission_is_required(self):
        raw = sample('N3M')
        self.assertEqual(assess_native(raw,'N3M',2)['classification'],'FORCED_AND_PASSED')
        self.assertNotEqual(assess_native(raw.replace('cpu=1','cpu=0'),'N3M',2)['classification'],'FORCED_AND_PASSED')
        missing = '\n'.join(line for line in raw.splitlines() if 'checkpoint=boundary-ready' not in line or 'result=2' not in line)
        self.assertEqual(assess_native(missing,'N3M',2)['classification'],'NOT_FORCED')

    def test_timeout_and_duplicate_commit_cannot_be_pass(self):
        raw = sample('N3M')
        for bad in (raw+'\n'+event(10,'boundary-timeout'), raw+'\n'+event(10,'mint-committed')):
            self.assertNotEqual(assess_native(bad,'N3M',2)['classification'],'FORCED_AND_PASSED')

    def test_retirement_does_not_mean_logout_committed(self):
        raw = sample().replace('checkpoint=transition-committed','checkpoint=transition-idempotent')
        self.assertEqual(assess_native(raw,'N1',2)['classification'],'NOT_FORCED')

    def test_wrong_policy_stage_and_missing_authentication_are_not_forced(self):
        for raw in (sample().replace('case=1 stage=0','case=1 stage=1'), sample().replace('checkpoint=registered','checkpoint=unregistered')):
            self.assertEqual(assess_native(raw,'N1',2)['classification'],'NOT_FORCED')

    def test_provenance_and_driver_timeout_reject_even_complete_markers(self):
        self.assertNotEqual(assess_native(sample(),'N1',2,identity=None)['classification'],'FORCED_AND_PASSED')
        for key in IDENTITY:
            identity = {**IDENTITY, key: False}
            self.assertNotEqual(assess_native(sample(),'N1',2,identity=identity)['classification'],'FORCED_AND_PASSED')
        result = assess_native(sample(),'N1',2,failure={'type':'TimeoutError'})
        self.assertNotEqual(result['classification'],'FORCED_AND_PASSED')

    def test_missing_outcome_cleanup_and_wrong_service_incarnation(self):
        raw = sample()
        for bad in (raw.replace('checkpoint=fixture-complete','checkpoint=unfinished'),
                    raw.replace('checkpoint=fixture-ipc-cleaned','checkpoint=not-cleaned'),
                    raw.replace('checkpoint=mint-response','checkpoint=unknown-outcome'),
                    raw.replace('checkpoint=mint-syscall-rejected pid=18 incarnation=30',
                                'checkpoint=mint-syscall-rejected pid=18 incarnation=99'),
                    raw+'\n[SESSION-E3-CLIENT] checkpoint=FAIL\n'+raw,
                    raw.replace('binding_vm=87','binding_vm=88')):
            self.assertNotEqual(assess_native(bad,'N1',2)['classification'],'FORCED_AND_PASSED')

    def test_reversed_order_and_malformed_identity_fail_closed(self):
        raw = sample()
        for bad in (raw.replace('checkpoint=transition-committed','checkpoint=SWAP')
                       .replace('checkpoint=mint-syscall-rejected','checkpoint=transition-committed')
                       .replace('checkpoint=SWAP','checkpoint=mint-syscall-rejected'),
                    raw.replace('cpu=1','cpu=bad')):
            self.assertNotEqual(assess_native(bad,'N1',2)['classification'],'FORCED_AND_PASSED')


if __name__ == '__main__': unittest.main()
