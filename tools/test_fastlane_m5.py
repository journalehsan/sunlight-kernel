"""Synthetic acceptance-parser controls; these never constitute native proof."""
import unittest
from fastlane_m5_assess import assess

IDENTITY = dict.fromkeys(('source', 'elf', 'iso', 'services'), True)


def synthetic():
    lines = []
    def row(prefix, **fields):
        lines.append(prefix + ' ' + ' '.join(f'{k}={v}' for k,v in fields.items()))
    def case(name, g, detail=0):
        row('[FL-M5]', case=name, generation=g, detail=detail)
    case('start',0)
    seq=0
    def trace(prefix, **fields):
        nonlocal seq
        seq+=1;row(prefix,seq=seq,**fields)
    names={**{g:('cycle',g-1) for g in range(1,9)}, **{g:('malformed',g-9) for g in range(9,13)},
           13:('mutation-during-copy',0),14:('close-before-ack',0),15:('revoke-during-copy',0),16:('cancel-wait',0),
           17:('producer-exit',0),18:('producer-exit',1),19:('consumer-exit-before-copy',0),20:('remote-retirement',0)}
    for g in range(1,21):
        p=100+g;c=200+g
        row('[FL-BIND]', op=3,state=3,generation=g,producer=p,consumer=c,producer_inc=p,consumer_inc=c,
            producer_vm=300+g,consumer_vm=400+g,issuer_inc=2,epoch=1)
        if g not in (16,17):
            row('[FL-TRANSFER]',event='publish',generation=g,message=0,slot_generation=1,pid=p,inc=p,vm=300+g)
        if g in (*range(1,9),13,14,20):
            row('[FL-M5-COPY]',generation=g,message=0,slot_generation=1,pid=c,bytes=256,stable=1,mixed=int(g==13),coherent=0,authentic=0)
        if g in (*range(1,9),13,20):
            row('[FL-TRANSFER]',event='ack',generation=g,message=0,slot_generation=1,pid=c,inc=c,vm=400+g)
        for notification in range(2 if g<=8 else int(g==16)):
            ident=dict(generation=g,consumer=c,inc=c,vm=400+g)
            row('[FL-BELL]',event='registered',cpu=1,**ident)
            row('[FL-BELL]',event='complete',cpu=0,blocked=1,result=9 if g==16 else 0,sequence=notification+1,**ident)
            row('[FL-BELL]',event='take',cpu=1,result=9 if g==16 else 0,sequence=notification+1,**ident)
            row('[FL-BELL-PARK]',parked=1,owner=255,queued=255,pins=0,ready=0,**ident)
        for region in range(3):
            frame=g*4096*3+region*4096
            trace('[FL-FRAME]',event='backing',generation=g,region=region,frame=frame)
            maps=[]
            for role,pid,vm in ((0,p,300+g),(1,c,400+g)):
                m=dict(generation=g,region=region,role=role,pid=pid,inc=pid,vm=vm,va=4096*(region+1))
                trace('[FL-MAP]',event='map',frame=frame,write=int(role==(1 if region==1 else 0)),nx=1,**m);maps.append(m)
            for m in maps:
                trace('[FL-MAP]',event='request',**m)
                tlb={k:m[k] for k in ('generation','region','vm','va')}
                targets=int(g==20 and region==2 and m['role']==1)*2
                trace('[FL-TLB]',event='request',request=seq+1,targets=targets,**tlb)
                request=seq
                if targets:trace('[FL-TLB]',event='ack',request=request,observed=request,targets=targets,cpu=1,flushed=1,**tlb)
                trace('[FL-TLB]',event='complete',request=request,targets=targets,**tlb)
                trace('[FL-MAP]',event='complete',**m)
            trace('[FL-FRAME]',event='free',generation=g,region=region,frame=frame)
        row('[FL-BELL-RESOURCE]',generation=g,active=0,waiters=0,receipts=0,pending=0)
        row('[FL-CENSUS]',checkpoint=500+g-1 if g<=8 else 520+g,objects=0,views=0,frames=0,shm=1304,tables=397)
        case(names[g][0],g,names[g][1])
    row('[FL-ACCESS]',pid=220,va=12288)
    row('[FL-PF]',pid=220,va=12288,error=4)
    lines.extend(('[FAULT] #14 diagnostic','[FAULT] Page Fault at diagnostic'))
    case('complete',0)
    return '\n'.join(lines)+'\n'


class Acceptance(unittest.TestCase):
    def good(self): return synthetic()
    def check_bad(self, text, **kwargs):
        self.assertEqual(assess(text,2,kwargs.get('identity',IDENTITY),kwargs.get('failure'))['status'],'FAIL')
    def test_complete_synthetic_control(self):
        self.assertEqual(assess(self.good(),2,IDENTITY)['errors'],[])
    def test_empty(self): self.check_bad('')
    def test_no_identity(self): self.check_bad(self.good(),identity=None)
    def test_execution_failure(self): self.check_bad(self.good(),failure={'reason':'deadline'})
    def test_missing_evidence_classes(self):
        for prefix in ('[FL-BIND]','[FL-TRANSFER]','[FL-M5-COPY]','[FL-BELL]','[FL-BELL-PARK]',
                       '[FL-BELL-RESOURCE]','[FL-CENSUS]','[FL-MAP]','[FL-FRAME]','[FL-TLB]','[FL-PF]'):
            with self.subTest(prefix=prefix):
                self.check_bad('\n'.join(l for l in self.good().splitlines() if not l.startswith(prefix)))
    def test_missing_single_physical_free(self):
        s=self.good();line=next(l for l in s.splitlines() if l.startswith('[FL-FRAME]') and 'event=free' in l)
        self.check_bad(s.replace(line+'\n','',1))
    def test_unacknowledged_remote_target(self):
        self.check_bad('\n'.join(l for l in self.good().splitlines() if not (l.startswith('[FL-TLB]') and 'event=ack' in l)))
    def test_duplicate_acknowledgement(self):
        s=self.good();line=next(l for l in s.splitlines() if l.startswith('[FL-TRANSFER]') and 'event=ack' in l)
        self.check_bad(s+line+'\n')
    def test_same_vm(self): self.check_bad(self.good().replace('consumer_vm=401','consumer_vm=301'))
    def test_stale_message(self): self.check_bad(self.good().replace('message=0','message=1',1))
    def test_coherence_overclaim(self): self.check_bad(self.good().replace('coherent=0','coherent=1',1))
    def test_live_terminal_resources(self): self.check_bad(self.good().replace('active=0 waiters=0','active=0 waiters=1'))
    def test_drift(self): self.check_bad(self.good().replace('checkpoint=500 objects=0 views=0 frames=0 shm=1304','checkpoint=500 objects=0 views=0 frames=0 shm=1305'))
    def test_duplicate_fields(self): self.check_bad(self.good().replace('case=complete','case=complete detail=0'))
    def test_extra_fault(self): self.check_bad(self.good()+'[FAULT] #13 extra\n')


if __name__=='__main__': unittest.main()
