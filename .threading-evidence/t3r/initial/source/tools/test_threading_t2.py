"""Synthetic receipts test parsing only; native proof comes from frozen guests."""
import unittest
from threading_t2_validate import assess

def complete(cpus=2):
    lines = [f'[T2] begin cpus={cpus} limits=32,96,512 cycles=3']
    stages = ['task-table'] + ['run-queue'] * 64 + ['environment-entries'] + ['string'] * 4 + ['capabilities', 'string', 'box', 'box', 'publication']
    for abi in range(2):
        lines += [f'[T2-FAULT] abi={abi} ordinal={n} stage={stage} rejected=ENOMEM' for n, stage in enumerate(stages, 1)]
        lines += [f'[T2] faults abi={abi} points=75 cleanup=exact']
    for n in range(3):
        lines += [f'[T2] pressure cycle={n} workers=96 vm_workers=32 observer=7 useful_cpus=0x3',
                  f'[T2] recovery cycle={n+1} heap=3032704 drift=0 unreaped_workers=0']
    lines += ['[T2] PASS faults_native=75 faults_linux=75 cycles=3 pressure=96 recovery=exact queues=unique T1_pins=preserved']
    return '\n'.join(lines) + '\n'

class Receipt(unittest.TestCase):
    def test_complete(self):
        self.assertTrue(assess(complete(), ['[T2] PASS'], 2))
    def test_missing_duplicate_wrong_point_and_leak(self):
        raw = complete()
        for bad in [raw.replace('ordinal=7 stage=run-queue', 'ordinal=8 stage=run-queue'),
                    raw.replace('stage=publication', 'stage=box'),
                    raw.replace('observer=7', 'observer=0'),
                    raw.replace('useful_cpus=0x3', 'useful_cpus=0x1'),
                    raw.replace('cycle=2 heap=3032704', 'cycle=2 heap=3032712'),
                    raw.replace('recovery=exact', 'recovery=unknown'),
                    raw.replace('[T2] recovery cycle=3', '[T2] recovery cycle=2'),
                    raw.rstrip(), raw + '[T2] PASS\n', raw + 'KERNEL PANIC\n',
                    '[T2] PASS\n']:
            with self.subTest(bad=bad[-100:]): self.assertFalse(assess(bad, ['[T2] PASS'], 2))
        self.assertFalse(assess(raw, ['[T2] PASS'], 4))
        self.assertFalse(assess(raw, []))
    def test_missing_each_record(self):
        lines=complete().splitlines(keepends=True)
        for index in range(len(lines)):
            self.assertFalse(assess(''.join(lines[:index]+lines[index+1:]), ['[T2] PASS'], 2), index)


class AcceptanceCoverage(unittest.TestCase):
    def test_pressure_does_not_substitute_for_named_races(self):
        from threading_t2_acceptance import assess, RACES
        result=assess({'native_matrix':{'2':{'admission':{'pass':True}}}})
        self.assertEqual(result['status'],'PARTIAL')
        for case in RACES:
            self.assertIn('forced_native_race/'+case,result['missing'])
    def test_unforced_and_unidentified_races_are_rejected(self):
        from threading_t2_acceptance import assess, RACES
        for classification in ('NOT_FORCED','FORCED_AND_FAILED','FORCED_AND_PASSED'):
            # No frozen image/serial identities, even when a label says PASS.
            result=assess({'forced_native_races':{case:{'pass':True,'cpus':2,
                          'classification':classification} for case in RACES}})
            self.assertTrue(all('forced_native_race/'+case in result['missing'] for case in RACES))

if __name__ == '__main__': unittest.main()
