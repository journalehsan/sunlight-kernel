"""Negative assessor controls only. These generated traces are not native evidence."""
import unittest
from fastlane_channel_validate import assess, CASES
IDENTITY=dict(source=True,elf=True,iso=True,services=True)
def trace():
    out=['[SMP] Phase-1 bring-up complete: 1 AP(s) online (2 total logical CPUs)']
    out += [f'[FL-USER] case={case} pid=10 result=0' for case in CASES]
    out += [f'[SESSION-E3] checkpoint={c} session=1 epoch=1 pid=6 incarnation=106 vm=206' for c in ('account-activated','login-proof-committed','registered','transition-committed')]
    out += [f'[FL-AUTH] op={op} pid=10 inc=100 vm=200 generation=1 result={code} cpu=0' for op,code in ((0,0),(1,0),(2,0),(3,3),(3,1),(3,2),(3,4),(23,0),(23,1))]
    for generation in range(1,5):
        for op,role,actor in ((0,2,6),(1,2,6),(2,2,10),(3,0,10),(3,1,11)):
            consumer=0 if op==0 else 11
            out.append(f'[FL-BIND] op={op} generation={generation} issuer=6 issuer_inc=106 issuer_vm=206 session=1 epoch=1 producer=10 producer_inc=110 producer_vm=210 consumer={consumer} consumer_inc={consumer+100 if consumer else 0} consumer_vm={consumer+200 if consumer else 0} actor={actor} actor_inc={actor+100} actor_vm={actor+200} role={role} state={op if op<3 else 3} cpu=0')
    for mode in range(1,5):
        seq=mode*100
        winner=1 if mode%2 else 2
        events=[('armed',0,0),('entered',1,0),('entered' if mode<=2 else 'exit-syscall',2,0),('lock-held',winner,0),('lock-contended',3-winner,0),('returned',winner,0),('returned',3-winner,4 if winner==2 else 0)]
        for i,(event,actor,result) in enumerate(events):
            pid=11 if (mode<=2 and actor==1) or (mode>=3 and actor==2) else 10
            out.append(f'[FL-RACE] seq={seq+i} event={event} generation={mode} mode={mode} actor={actor} pid={pid} inc={pid+100} vm={pid+200} result={result} cpu={actor%2}')
        for actor in (winner,3-winner):
            terminal=1 if mode<=2 else 2
            out.append(f'[FL-OUTCOME] seq={seq+10+actor} generation={mode} actor={actor} state=4 terminal={terminal} producer=0 consumer=0 cpu={actor%2}')
        out.append(f'[FL-USER] case=contended-race pid=10 result={mode}')
    out+=['[SCHED] process_reaped pid=10 name=fixture','[SYSCALL] exec: path not found: /missing-channel-exec','exit: /tmp/session-race-probe FL -> 0']
    return '\n'.join(out)
class Controls(unittest.TestCase):
    def test_synthetic_control_is_accepted_by_parser_only(self):
        self.assertEqual(assess(trace(),2,IDENTITY)['errors'],[])
    def test_every_required_case_is_required(self):
        for case in CASES:
            with self.subTest(case=case):
                raw=trace().replace(f'[FL-USER] case={case} pid=10 result=0','')
                self.assertEqual(assess(raw,2,IDENTITY)['status'],'FAIL')
    def test_missing_identity_rejected(self):
        self.assertEqual(assess(trace(),2)['status'],'FAIL')
    def test_driver_failure_rejected(self):
        self.assertEqual(assess(trace(),2,IDENTITY,'timeout')['status'],'FAIL')
    def test_missing_contention_not_pass(self):
        self.assertEqual(assess(trace().replace('event=lock-contended','event=not-forced'),2,IDENTITY)['status'],'FAIL')
    def test_same_cpu_not_concurrency(self):
        self.assertEqual(assess(trace().replace('cpu=1','cpu=0'),2,IDENTITY)['status'],'FAIL')
    def test_wrong_winner_rejected(self):
        self.assertEqual(assess(trace().replace('event=lock-held generation=1 mode=1 actor=1','event=lock-held generation=1 mode=1 actor=2'),2,IDENTITY)['status'],'FAIL')
    def test_second_terminal_rejected(self):
        self.assertEqual(assess(trace().replace('terminal=2','terminal=1'),2,IDENTITY)['status'],'FAIL')
    def test_duplicate_fields_rejected(self):
        self.assertEqual(assess(trace()+'\n[FL-USER] case=complete case=complete result=0',2,IDENTITY)['status'],'FAIL')
    def test_guest_fault_rejected(self):
        self.assertEqual(assess(trace()+'\nPANIC',2,IDENTITY)['status'],'FAIL')
if __name__=='__main__':unittest.main()

class BindingControls(unittest.TestCase):
    def test_kernel_bindings_required(self):
        raw='\n'.join(l for l in trace().splitlines() if not l.startswith('[FL-BIND]'))
        self.assertEqual(assess(raw,2,IDENTITY)['status'],'FAIL')
    def test_foreign_successful_attachment_rejected(self):
        raw=trace().replace('actor=11 actor_inc=111 actor_vm=211 role=1','actor=12 actor_inc=112 actor_vm=212 role=1')
        self.assertEqual(assess(raw,2,IDENTITY)['status'],'FAIL')
    def test_duplicate_successful_attachment_rejected(self):
        raw=trace();raw+='\n'+next(l for l in raw.splitlines() if l.startswith('[FL-BIND] op=3'))
        self.assertEqual(assess(raw,2,IDENTITY)['status'],'FAIL')
    def test_session_epoch_substitution_rejected(self):
        self.assertEqual(assess(trace().replace('epoch=1 producer=','epoch=2 producer='),2,IDENTITY)['status'],'FAIL')

class BoundedNaturalContentionControls(unittest.TestCase):
    def test_missed_window_alone_is_not_coverage(self):
        raw=trace().replace('event=lock-contended generation=1', 'event=lock-missed generation=1')
        self.assertIn('one qualifying concurrent attempt required 1', assess(raw,2,IDENTITY)['errors'])

    def test_bounded_miss_then_real_contention(self):
        original=trace()
        extra=[]
        for line in original.splitlines():
            if line.startswith(('[FL-RACE]', '[FL-OUTCOME]')) and 'generation=1 ' in line:
                import re
                line=re.sub(r'seq=(\d+)',lambda m:'seq='+str(int(m[1])+1000),line)
                extra.append(line.replace('generation=1 ', 'generation=5 '))
            elif line.startswith('[FL-BIND]') and 'generation=1 ' in line:
                extra.append(line.replace('generation=1 ', 'generation=5 '))
        raw=original.replace('event=lock-contended generation=1', 'event=lock-missed generation=1')
        raw+='\n[FL-USER] case=contention-missed pid=10 result=1\n'+'\n'.join(extra)
        data=assess(raw,2,IDENTITY)
        self.assertEqual(data['errors'],[])
        self.assertEqual(len(data['race_attempts']),5)
        self.assertEqual(sum(r['qualified'] for r in data['race_attempts']),4)

    def test_missed_attempt_must_be_reported(self):
        raw=trace().replace('event=lock-contended generation=1', 'event=lock-missed generation=1')
        self.assertIn('missing contention-window miss receipt',assess(raw,2,IDENTITY)['errors'])

    def test_same_mode_cannot_rearm_same_generation(self):
        raw=trace();arm=next(line for line in raw.splitlines() if 'event=armed generation=1 ' in line)
        self.assertIn('rearmed generation 1',assess(raw+'\n'+arm,2,IDENTITY)['errors'])

    def test_race_actor_must_match_exact_binding(self):
        raw=trace().replace('actor=1 pid=11 inc=111 vm=211','actor=1 pid=12 inc=112 vm=212',1)
        self.assertIn('race actor exact binding',assess(raw,2,IDENTITY)['errors'])
