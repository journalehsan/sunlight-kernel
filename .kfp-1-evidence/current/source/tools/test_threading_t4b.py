"""Synthetic parser controls are not native evidence."""
import unittest
from test_threading_t3r import synthetic as topology
from threading_t4b_validate import assess_races, guest_assess


def synthetic():
    text=topology()
    for case in range(11):
        actors=32 if case==8 else 3
        for r in range(actors):
            text+=f'[T4B] created case={case} role={r} tid={100+r} inc={1000+r} vm=1 stack=0x{(r+1)*4096:x} tls=0x{(r+1)*16:x}\n'
        roles=list(range(32)) if case==8 else [0]
        result=2 if case in (1,9) else 0
        for r in roles:
            for seq in range(1,17 if case==9 else 3 if case==8 and r==1 else 2):
                text+=f'[T4B] registered case={case} role={r} seq={seq} tid={100+r} inc={1000+r}\n'
                text+=f'[T4B] terminal case={case} role={r} seq={seq} tid={100+r} inc={1000+r} result={result}\n'
        reg=33 if case==8 else 16 if case==9 else 1
        selected=33 if case==8 else 1 if case in (0,2,4,7) else 0
        text+=f'[T4B] case={case} registered={reg} completed={reg} selected={selected} result={result} dispatch0=0x{3 if case==7 else 1:x} heap=100 frames=1000 active=0 deadlines=0 receipts=0 queues=0 pins=0 refs=3\n'
        text+=f'[T4B] progress case={case} ticks=1 userspace_selected={32 if case==8 else 0}\n'
    text+='[T4B] contender role=0 tid=100 inc=1000 worker_cpu=1 owner_cpu=0\n'
    for role in (0,64):
        for limit in (0,1,4,1,4,1,32,1):
            text+=f'[T4B] wake case=8 role={role} limit={limit} selected={30 if role==0 and limit==32 else 0}\n'
    return text+'[T4B] PASS cpus=2 cases=11 resources=stable\n'


class Receipts(unittest.TestCase):
    def test_restart_cannot_hide_futex_failure(self):
        text=topology()+"std-complete\n"
        self.assertTrue(guest_assess(text,["std-complete"],2))
        self.assertFalse(guest_assess("native futex wait failed\n"+text,["std-complete"],2))
        self.assertFalse(guest_assess("[T4B-DIAG] missing-result\n"+text,["std-complete"],2))

    def test_unexpected_panic_rejected(self):
        text=topology()+"std-complete\n"
        self.assertFalse(guest_assess("thread 'worker' panicked at x:1:1:\nunexpected\n"+text,["std-complete"],2))
        self.assertTrue(guest_assess("thread 'worker' panicked at x:1:1:\nnative worker panic containment probe\n"+text,["std-complete"],2))

    def test_positive(self):
        self.assertTrue(assess_races(synthetic(),2))

    def test_reject_missing_or_corrupt_proof(self):
        text=synthetic()
        variants=[text.replace('completed=16','completed=15'),text.replace('seq=16','seq=15'),
                  text.replace('inc=1000 result','inc=999 result',1),text.replace('active=0','active=1',1),
                  text.replace('ticks=1','ticks=0',1),text.replace('selected=32','selected=33',1),
                  text.replace('dispatch0=0x3','dispatch0=0x1'),text.replace('frames=1000','frames=999',1),
                  text.replace('result=2','result=0'),text+'[EXCEPTION]\n',text.replace('worker_cpu=1','worker_cpu=0'),
                  '\n'.join(l for l in text.splitlines() if 'terminal case=6 ' not in l)+'\n']
        for i,variant in enumerate(variants):
            with self.subTest(i=i): self.assertFalse(assess_races(variant,2))

if __name__=='__main__': unittest.main()
