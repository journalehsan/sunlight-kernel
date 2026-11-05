"""Adversarial parser tests. Synthetic streams are not native evidence."""
import unittest
from fastlane_f11b0g_validate import assess_g
from test_fastlane_f11b0f import IDENTITY


def fixture(cpus=2):
    lines = []
    for epoch in range(1, 5):
        if epoch > 1:
            lines.append(f'[GRANT-G] cycle-commit epoch={epoch} pid=62704 incarnation=10 vm=11 request={epoch} count=2 cpu=0')
        for service, pid in [('uac',20), ('vfs',21)]:
            lines.append(f'[GRANT-G] service-ack epoch={epoch} service={service} pid={pid} incarnation=30 vm=31 caller=62704 caller_inc=10 request={epoch} records=0 active_or_boot_handles=0 heap_used=100 boundary=handler-returned cpu=0')
        lines.append(f'[GRANT-G] boundary epoch={epoch} services=uac,vfs scope=completed-fixture-operations sessiond=UNMEASURED fixture_grants=0 cpu=0')
    lines += ['[GRANT-G] exec-reply checkpoint=commit pid=62709 incarnation=40 vm=41 request=1 count=2 cpu=0',
              '[GRANT-G] exec-reply checkpoint=cancel pid=62709 incarnation=40 vm=41 request=1 retired=2 terminal=deadline inventory=0 cpu=0',
              '[GRANT-G] exec-reply checkpoint=exec pid=62709 incarnation=40 old_vm=41 new_vm=42 retired=2 exec_retired=0 retirement=ipc-cancel-before-exec cpu=0',
              '[GRANT-G] exec-reply checkpoint=reaped pid=62709 incarnation=40 cpu=0',
              '[GRANT-G] exec-reply checkpoint=late-settlement pid=62709 incarnation=40 request=1 inventory=0 cpu=0',
              '[GRANT-G] cycles COMPLETE epochs=4 real_cycles=3 service_acks=8',
              f'[HISTORY-G] armed scope=global-production-broker capacity=1024 production_capacity=65536 history=1023 record_bytes=8 baseline_live=50 sentinel=usable oldest=denied entropy=production identity_domain=kernel-boot reset=UNSUPPORTED cpus={cpus}']
    lines += [f'[HISTORY-G] ready cpu={cpu} locks=none' for cpu in range(cpus)]
    lines += [f'[HISTORY-G] attempt cpu={cpu} result='+('last-slot newest=denied recycled_slot=retired' if cpu == 0 else 'TokenHistoryFull partial=0') for cpu in range(cpus)]
    lines.append(f'[HISTORY-G] COMPLETE PASS ready_mask={(1<<cpus)-1} done_mask={(1<<cpus)-1} winners=1 rejected={cpus-1} history=1024 capacity=1024 retained_bytes=8192 final_live=50 oldest=denied newest=denied existing=retired rollback=zero-partial recovery=none syscall_error_transport=NOT_RUN')
    return '\n'.join(lines)


class ClosureParser(unittest.TestCase):
    def test_supported_cpu_counts_and_scope(self):
        for cpus in (1,2,4,12):
            result = assess_g(fixture(cpus), cpus, IDENTITY)
            self.assertEqual(result['errors'], [])
            self.assertEqual(result['global_reconciliation'], 'PARTIAL')
            self.assertEqual(result['syscall_error_transport'], 'NOT_RUN')

    def test_missing_duplicate_and_scope_inflation_rejected(self):
        raw = fixture()
        for before, after in [('entropy=production','entropy=synthetic'), ('oldest=denied','oldest=allowed'),
                              ('syscall_error_transport=NOT_RUN','syscall_error_transport=PASS'),
                              ('fixture_grants=0','fixture_grants=1'), ('sessiond=UNMEASURED','sessiond=PASS'),
                              ('new_vm=42','new_vm=41'), ('incarnation=40','incarnation=0'),
                              ('done_mask=3','done_mask=1'), ('result=TokenHistoryFull','result=last-slot'),
                              ('final_live=50','final_live=51'), ('capacity=1024','capacity=1025'),
                              ('service=vfs','service=uac'), ('caller_inc=10','caller_inc=0'),
                              ('cpu=1','cpu=2'), ('retired=2','retired=1'),
                              ('history=1023','history=1022'), ('epoch=4','epoch=3'), ('terminal=deadline','terminal=unknown'),
                              ('request=1 inventory=0','request=2 inventory=0'), ('retired=2 terminal=deadline','retired=0 terminal=deadline'),
                              ('sentinel=usable','sentinel=unknown'), ('COMPLETE PASS','COMPLETE FAILED'),
                              ('count=2','count=1'), ('records=0','records=unknown')]:
            with self.subTest(before=before): self.assertTrue(assess_g(raw.replace(before, after),2,IDENTITY)['errors'])
        for line in raw.splitlines():
            self.assertTrue(assess_g(raw.replace(line,'',1),2,IDENTITY)['errors'], line)
        self.assertTrue(assess_g(raw+'\n'+raw.splitlines()[0],2,IDENTITY)['errors'])
        self.assertTrue(assess_g(raw.replace('cpu=1','cpu=1 cpu=1'),2,IDENTITY)['errors'])

    def test_real_transport_rollback_contract(self):
        raw = fixture().split('[HISTORY-G] armed')[0] + "\n".join([
            '[HISTORY-GT] prefill history=1023 capacity=1024 authority_added=0 entropy=production cpu=0',
            '[HISTORY-GT] rollback attempt=1 pid=62704 incarnation=10 vm=11 request=40 history=1024 vfs_partial=0 proof=retained error=TokenHistoryFull cpu=0',
            '[HISTORY-GT] client error=TokenHistoryFull caps=0',
            '[HISTORY-GT] rollback attempt=2 pid=62704 incarnation=10 vm=11 request=41 history=1024 vfs_partial=0 proof=retained error=TokenHistoryFull cpu=0',
            '[HISTORY-GT] client error=TokenHistoryFull caps=0',
            '[HISTORY-GT] COMPLETE PASS capacity=1024 history=1024 attempts=2 partial=0 proof=exec-retired error_transport=explicit exhaustion=permanent'])
        self.assertEqual(assess_g(raw,2,IDENTITY,transport=True)['errors'], [])
        for before, after in [('vfs_partial=0','vfs_partial=1'),('proof=retained','proof=lost'),
                              ('attempt=2','attempt=1'),('error=TokenHistoryFull','error=Denied'),
                              ('error_transport=explicit','error_transport=unknown'),('history=1023','history=1022')]:
            self.assertTrue(assess_g(raw.replace(before,after),2,IDENTITY,transport=True)['errors'])

    def test_identity_order_and_native_failure(self):
        raw = fixture()
        self.assertTrue(assess_g(raw,2)['errors'])
        self.assertTrue(assess_g(raw,2,{**IDENTITY,'elf_matches':False})['errors'])
        lines = raw.splitlines(); lines[0],lines[2]=lines[2],lines[0]
        self.assertTrue(assess_g('\n'.join(lines),2,IDENTITY)['errors'])
        self.assertTrue(assess_g(raw+'\nPANIC',2,IDENTITY)['errors'])


if __name__ == '__main__': unittest.main()
