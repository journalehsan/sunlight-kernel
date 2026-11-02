"""Synthetic acceptance controls; no synthetic result counts as native evidence."""
import unittest
from fastlane_m51_assess import assess
from test_fastlane_m5 import synthetic as old_synthetic
from fastlane_channel_validate import rows
IDENTITY=dict.fromkeys(('source','elf','iso','services'),True)
def synthetic():
    lines=['[SMP] Limine MP response: 2 CPU(s), BSP LAPIC ID=0','[SMP] AP1 (LAPIC 1) online — TSS loaded, LAPIC timer armed','[SCHED] Per-core work-stealing scheduler: 2 online core(s)']
    def row(prefix,**kw):lines.append(prefix+' '+' '.join(f'{k}={v}' for k,v in kw.items()))
    for line in old_synthetic().splitlines():
        if line.startswith(('[FL-BIND]','[FL-MAP]','[FL-FRAME]','[FL-TLB]','[FL-BELL-RESOURCE]')):
            r=rows(line,line.split(' ')[0]+' ')[0]
            if int(r['generation'])<=15:
                if line.startswith('[FL-TLB]') and r['generation']=='15' and r['region']=='2' and r['vm']=='415':
                    line=line.replace('targets=0','targets=2')
                    if r['event']=='complete':
                        row('[FL-TLB]',**{**r,'event':'ack','targets':2,'cpu':1,'flushed':1,'observed':r['request'],'seq':r['seq']})
                lines.append(line)
    cases=[('cycle',1,1),('cycle',2,16),('cycle',3,1000),('cycle',4,10000),('slow-producer',5,1),('cancel',6,0),*[('malformed',7+i,i) for i in range(4)],('mutation',11,16),('revoke-copy',12,16),('consumer-exit-copy',13,16),('close-ack',14,16),('producer-exit',15,16)]
    # Assign strict integer trace indices after inserting the remote ACK.
    physical=[]
    for i,line in enumerate(lines):
        if line.startswith(('[FL-MAP]','[FL-FRAME]','[FL-TLB]')):
            import re
            line=re.sub(r'\bseq=\S+',f'seq={i+1}',line)
        physical.append(line)
    lines=physical
    row('[FL-M51]',case='start',generation=0,count=0)
    for case,g,detail in cases:
        count=detail if g<=6 else (17 if g==11 else 16)
        published=17 if g>6 else count
        for seq in range(published):
            common=dict(generation=g,message=seq,slot_generation=seq//16+1)
            row('[FL-REPEAT]',event='control',op=1537,pid=100+g,inc=100+g,vm=300+g,**common)
            row('[FL-REPEAT]',event='publish',pid=100+g,inc=100+g,vm=300+g,**common)
            row('[FL-M51-MSG]',event='producer',pid=100+g,bytes=seq%257,**common)
            if seq<count:
                row('[FL-REPEAT]',event='control',op=1538,pid=200+g,inc=200+g,vm=400+g,**common)
                row('[FL-REPEAT]',event='control',op=1539,pid=200+g,inc=200+g,vm=400+g,**common)
                row('[FL-REPEAT]',event='ack',pid=200+g,inc=200+g,vm=400+g,**common)
                row('[FL-M51-MSG]',event='consumer',pid=200+g,bytes=seq%257,coherent=0,authentic=0,**common)
        if case in ('slow-producer','cancel'):
            common=dict(generation=g,consumer=200+g,inc=200+g,vm=400+g)
            row('[FL-BELL]',event='registered',**common)
            row('[FL-BELL]',event='complete',result=0 if case=='slow-producer' else 9,sequence=1,**common)
            row('[FL-BELL]',event='take',result=0 if case=='slow-producer' else 9,sequence=1,**common)
            row('[FL-BELL-PARK]',parked=1,owner=255,queued=255,pins=0,ready=0,**common)
        if case=='cycle':
            for n in range(detail//16):row('[FL-M51]',case='full',generation=g,count=n*16)
        row('[FL-M51]',case=case,generation=g,count=detail)
        row('[FL-CENSUS]',checkpoint=599+g,objects=0,views=0,frames=0,shm=1304,tables=389)
    row('[FL-M51-TERMINAL]',generation=13,operation='copy-caller',exit=137,notify=4,late_reply=0)
    row('[FL-M51]',case='complete',generation=0,count=0)
    return '\n'.join(lines)+'\n'
class Acceptance(unittest.TestCase):
    @classmethod
    def setUpClass(cls):cls.raw=synthetic()
    def test_complete(self):self.assertEqual(assess(self.raw,2,IDENTITY)['errors'],[])
    def bad(self,raw,identity=IDENTITY):self.assertEqual(assess(raw,2,identity)['status'],'FAIL')
    def test_no_identity(self):self.bad(self.raw,None)
    def test_missing_classes(self):
        for prefix in ('[FL-M51-MSG]','[FL-REPEAT]','[FL-CENSUS]','[FL-FRAME]','[FL-MAP]','[FL-TLB]','[FL-BIND]'):
            with self.subTest(prefix=prefix):self.bad('\n'.join(l for l in self.raw.splitlines() if not l.startswith(prefix)))
    def test_duplicates(self):
        for event in ('publish','ack'):
            line=next(l for l in self.raw.splitlines() if l.startswith('[FL-REPEAT] event='+event))
            self.bad(self.raw+line+'\n')
    def test_stale_sequence(self):self.bad(self.raw.replace('message=0','message=1',1))
    def test_generation(self):self.bad(self.raw.replace('slot_generation=1','slot_generation=99',1))
    def test_overclaim(self):self.bad(self.raw.replace('coherent=0','coherent=1',1))
    def test_missing_remote_ack(self):self.bad('\n'.join(l for l in self.raw.splitlines() if not(l.startswith('[FL-TLB]') and 'event=ack' in l)))
    def test_missing_free(self):
        line=next(l for l in self.raw.splitlines() if l.startswith('[FL-FRAME]') and 'event=free' in l)
        self.bad(self.raw.replace(line+'\n','',1))
    def test_order(self):
        lines=self.raw.splitlines();a=next(i for i,l in enumerate(lines) if l.startswith('[FL-REPEAT] event=publish'));b=next(i for i,l in enumerate(lines) if l.startswith('[FL-REPEAT] event=ack'));lines[a],lines[b]=lines[b],lines[a];self.bad('\n'.join(lines))
if __name__=='__main__':unittest.main()
