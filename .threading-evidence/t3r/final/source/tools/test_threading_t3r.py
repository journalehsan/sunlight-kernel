"""Host-only parser controls. The positive transcript is explicitly synthetic."""
import unittest
from threading_t3r_receipt import assess_migration


def synthetic():
    lines=[]
    for c in range(2):
        lines += [f'[T3R-FW] slot={c} processor={c} apic={c} bsp=0',
                  f'[CPU-ID] logical={c} apic={c} bsp={str(c == 0).lower()}']
    for c in range(2):
        lines += [f'[CPU-ID] {"bsp" if c == 0 else "online"} logical={c} apic={c} tss=0x{100+c:x} ist_df=0x{200+c:x} ist_nmi=0x{300+c:x}']
    def add(e,s): lines.append('[T3R] '+e+' '+s)
    add('begin','cpus=2 owner=10 inc=10 root=4096 generation=1')
    for r in range(2):
        add('created',f'role={r} owner=10 owner_inc=10 tid={11+r} inc={11+r} root=4096 generation=1 slot={1+r} stack=0x{500+r:x} tls=0x{600+r:x} queued=1')
    def location(r,c):
        return f'tid={11+r} inc={11+r} cpu={c} apic={c} stack=0x{500+r:x} tls=0x{600+r:x} tss=0x{100+c:x} rsp0=0x{500+r:x} df=0x{200+c:x} nmi=0x{300+c:x}'
    add('dispatch','role=0 '+location(0,0)+' remote_pin=false')
    add('dispatch','role=1 '+location(1,0)+' remote_pin=false')
    add('execution',location(0,0))
    add('request','tid=11 inc=11 source=0 destination=1 blocked=true pinned=false')
    add('wake','tid=11 inc=11 from=0 queued=1 request=true wait=cleared pinned=false')
    add('dispatch','role=0 '+location(0,1)+' remote_pin=false')
    add('execution',location(0,1))
    add('state','tid=11 inc=11 tls=true fs=true gpr=true x87=true xmm=true mxcsr=true canary=true')
    for r in range(2):
        add('retire',f'role={r} tid={11+r} inc={11+r} count=1')
        add('reaped',f'role={r} tid={11+r} inc={11+r} stack=0 tls=0 current=0 queued=0 pins=0 pending=0 exit=0')
    add('resources','heap_base=1000 heap_final=1200 cache_bytes=200 frame_base=400 frame_final=400 workers=0 reservations=0 stacks=1 fd_refs=3 handoff_pins=0 handoff_waiters=0 user_backing=owner_static pulses=1,2,3')
    add('PASS','cpus=2 executions=0,1 physical_distinct=true')
    return '\n'.join(lines)+'\n'


class Controls(unittest.TestCase):
    def test_synthetic_positive(self):
        self.assertTrue(assess_migration(synthetic(),2))

    def test_fail_closed(self):
        text=synthetic()
        variants={
            'request_without_execution':'\n'.join(l for l in text.splitlines() if not l.startswith('[T3R] execution'))+'\n',
            'wrong_owner_incarnation':text.replace('owner_inc=10','owner_inc=99',1),
            'wrong_thread_incarnation':text.replace('[T3R] execution tid=11 inc=11','[T3R] execution tid=11 inc=99',1),
            'same_physical_cpu':text.replace('cpu=1 apic=1','cpu=1 apic=0'),
            'missing_destination':'\n'.join(l for l in text.splitlines() if not (l.startswith('[T3R] execution') and 'cpu=1' in l))+'\n',
            'invalid_mapping':text.replace('logical=1 apic=1 bsp=false','logical=1 apic=9 bsp=false'),
            'wrong_rsp0':text.replace('rsp0=0x1f4','rsp0=0x1f5'),
            'wrong_ist':text.replace('nmi=0x12d','nmi=0x12c'),
            'missing_tls':text.replace('tls=true ',''),
            'inconsistent_fp':text.replace('x87=true','x87=false'),
            'missing_state':'\n'.join(l for l in text.splitlines() if not l.startswith('[T3R] state'))+'\n',
            'missing_retirement':'\n'.join(l for l in text.splitlines() if not l.startswith('[T3R] retire'))+'\n',
            'duplicate_retirement':text.replace('count=1','count=2',1),
            'abandoned_handoff':text.replace('handoff_waiters=0','handoff_waiters=2'),
            'leaked_resources':text.replace('heap_final=1200','heap_final=1201'),
            'no_worker_progress':text.replace('pulses=1,2,3','pulses=1,0,3'),
            'stale_vm_generation':text.replace('generation=1 slot=1','generation=2 slot=1'),
            'panic':text+'KERNEL PANIC\n',
            'truncated':text.rstrip('\n'),
        }
        for name,broken in variants.items():
            with self.subTest(name=name): self.assertFalse(assess_migration(broken,2))
