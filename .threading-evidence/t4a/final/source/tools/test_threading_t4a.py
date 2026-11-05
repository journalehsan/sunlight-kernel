"""Parser negative controls use synthetic text; they are never native evidence."""
import unittest
from test_threading_t3r import synthetic as topology
from threading_t4a_validate import assess_races


def synthetic():
    text = topology()+"[T4A] unsupported SHM aliases=2 rejected=true backing_released=true\n"
    pid = 10
    for case in range(34):
        c = case % 17
        count = 32 if c >= 15 else 3 if c == 7 else 2
        roles = range(32) if count == 32 else range(count)
        for role in roles:
            pid += 1
            text += f'[T4A] created case={case} role={role} tid={pid} inc={pid} stack=0x{pid*4096:x} tls=0x{pid*16:x}\n'
        if c in range(1,6):
            text += f'[T4A] ordering case={case} point={c-1} waiter_cpu=0 waker_cpu=1 entered=1 completed=1\n'
        if c == 15:
            limits = ((0,0,0),(1,1,1),(4,4,5),(2**32-1 if case==15 else 2**31-1,27,32),(1,0,32))
            for step,(limit,actual,total) in enumerate(limits):
                text += f'[T4A] cardinality case={case} step={step} limit={limit} actual={actual} selected_total={total} resumed=0 remaining={32-total} unique=true\n'
        reg = 0 if c in (0,1) or case==25 else 2 if c==7 else 128 if c==14 else 32 if c>=15 else 1
        wakes = 128 if c==14 else reg
        live = 1 if c==5 else 0
        mask = 3 if c==14 else 1
        text += f'[T4A] case={case} registered={reg} completed={reg} wakes={wakes} live_wakes={live} dispatch0=0x{mask:x} heap=100 frames=1000 stale=0 pins=0 queues=0 ownership=ok\n'
    return text+'[T4A] PASS cpus=2 cases=34 waiter_records=0 resources=stable\n'


class Receipts(unittest.TestCase):
    def test_synthetic_positive(self):
        self.assertTrue(assess_races(synthetic(),2))

    def test_reject_missing_or_corrupt_proof(self):
        text = synthetic()
        variants = [
            text.replace('limit=1 actual=1','limit=1 actual=2',1),
            text.replace('limit=0 actual=0','limit=0 actual=32',1),
            text.replace('resumed=0 remaining=31','resumed=2 remaining=31',1),
            text.replace('completed=128','completed=127',1),
            text.replace('dispatch0=0x3','dispatch0=0x1',1),
            text.replace('waiter_cpu=0 waker_cpu=1','waiter_cpu=0 waker_cpu=0',1),
            text.replace('entered=1 completed=1','entered=0 completed=1',1),
            text.replace('stale=0','stale=1',1),
            text.replace('frames=1000','frames=999',1),
            text+'[EXCEPTION]\n',
            '\n'.join(l for l in text.splitlines() if 'cardinality' not in l)+'\n',
            '\n'.join(l for l in text.splitlines() if not l.startswith('[T4A] ordering'))+'\n',
            '\n'.join(l for l in text.splitlines() if not l.startswith('[T4A] created case=32 role=31 '))+'\n',
        ]
        for i,variant in enumerate(variants):
            with self.subTest(control=i):
                self.assertFalse(assess_races(variant,2))
