"""Strict native M5.1 count, ordering, identity and physical-retirement joins."""
from fastlane_channel_validate import rows
import re

def assess(raw, cpus, identity=None, failure=None):
    errors=[]
    def need(ok,why):
        if not ok: errors.append(why)
    def n(r,k):
        try:return int(r[k])
        except (KeyError,ValueError,TypeError):errors.append('missing integer '+k);return -1
    need(cpus in (2,4,12),'topology')
    need(f'[SMP] Limine MP response: {cpus} CPU(s)' in raw and f'[SCHED] Per-core work-stealing scheduler: {cpus} online core(s)' in raw,'native online topology')
    need({int(x) for x in re.findall(r'\[SMP\] AP(\d+) .* online',raw)}==set(range(1,cpus)),'online AP identities')
    need(identity is not None and all(identity.get(k) is True for k in ('source','elf','iso','services')),'identity')
    need(failure is None,'execution failure')
    need(not any(v in raw for v in ('PANIC','panicked at','checkpoint=FAIL','[FL-BELL-DEADLINE]','[FAULT]')),'guest failure')
    cases=rows(raw,'[FL-M51] ');bindings=[r for r in rows(raw,'[FL-BIND] ') if r.get('op')=='3' and r.get('state')=='3']
    gens={r.get('generation'):r for r in bindings}
    need(len(bindings)==len(gens)==15,'15 persistent channels')
    cycles=[r for r in cases if r.get('case')=='cycle']
    need(sorted(n(r,'count') for r in cycles)==[1,16,1000,10000],'cycle count coverage')
    need(len([r for r in cases if r.get('case')=='complete'])==1,'terminal checkpoint')
    expected={'start':[0],'cycle':[1,16,1000,10000],'slow-producer':[1],'cancel':[0],
              'malformed':[0,1,2,3],'mutation':[16],'revoke-copy':[16],'close-ack':[16],
              'consumer-exit-copy':[16],'producer-exit':[16],'complete':[0]}
    need(set(r.get('case') for r in cases)==set(expected)|{'full'},'case set')
    for case,counts in expected.items():
        need(sorted(n(r,'count') for r in cases if r.get('case')==case)==counts,'case coverage '+case)
    need(len([r for r in cases if r.get('case')=='slow-producer'])==1 and len([r for r in cases if r.get('case')=='cancel'])==1,'pending lifecycle coverage')
    assigned=[r for r in cases if r.get('case') in ('cycle','slow-producer','cancel','malformed','mutation','revoke-copy','close-ack','consumer-exit-copy','producer-exit')]
    need(len({r.get('generation') for r in assigned})==15 and {r.get('generation') for r in assigned}==set(gens),'channel case join')
    events=rows(raw,'[FL-REPEAT] ');copies=rows(raw,'[FL-M51-MSG] ')
    for case in assigned:
        g=case.get('generation');b=gens.get(g,{});count=n(case,'count')
        adversarial=case.get('case') in ('malformed','mutation','revoke-copy','close-ack','consumer-exit-copy','producer-exit')
        published=17 if adversarial else count
        count=(17 if case.get('case')=='mutation' else 16) if adversarial else count
        need(b.get('producer')!=b.get('consumer') and b.get('producer_vm')!=b.get('consumer_vm'),'independent peers')
        for role in ('producer','consumer'):
            need(all(n(b,k)>0 for k in (role,role+'_inc',role+'_vm')),'authenticated identity')
        for event,role in (('publish','producer'),('ack','consumer')):
            es=[r for r in events if r.get('generation')==g and r.get('event')==event]
            ms=[r for r in copies if r.get('generation')==g and r.get('event')==role]
            need([n(r,'message') for r in es]==list(range(published if event=='publish' else count)),event+' exact ordered count')
            need([n(r,'message') for r in ms]==list(range(published if role=='producer' else count)),role+' independent exact count')
            for r in es:
                need(all(r.get(k)==b.get(role+suffix) for k,suffix in [('pid',''),('inc','_inc'),('vm','_vm')]),'event role binding')
                need(n(r,'slot_generation')==n(r,'message')//16+1,'checked generation')
            for r in ms:
                need(r.get('pid')==b.get(role) and n(r,'bytes')==n(r,'message')%257,'bounded contents receipt')
                if role=='consumer':need(r.get('coherent')==r.get('authentic')=='0','integrity contract')
        ev=[r for r in events if r.get('generation')==g]
        positions={}
        for i,r in enumerate(ev):
            positions.setdefault((n(r,'message'),r.get('event'),r.get('op')),[]).append(i)
        for r in ev:
            seq=n(r,'message')
            need(n(r,'slot_generation')==seq//16+1,'control generation')
            if r.get('event')=='control':
                role='producer' if r.get('op') in ('1536','1537','1540') else 'consumer'
                need(all(r.get(k)==b.get(role+suffix) for k,suffix in [('pid',''),('inc','_inc'),('vm','_vm')]),'control role binding')
        for seq in range(count):
            def pos(event,op=None):
                found=positions.get((seq,event,str(op) if op is not None else None),[])
                need(len(found)==1,'unique '+event+' '+str(op));return found[0] if found else -1
            p=pos('publish');a=pos('control',1538);c=pos('control',1539);ack=pos('ack')
            need(p<a<c<ack,'acquire/copy/ack order')
            reserves=positions.get((seq,'control','1537'),[])
            need(any(r<p for r in reserves),'reservation before publication')
            if seq>=16:
                old=positions.get((seq-16,'ack',None),[])
                need(len(old)==1 and any(old[0]<r<p for r in reserves),'reuse after ack')
    need(len([r for r in events if r.get('event')=='publish'])==11171,'total publications')
    need(len([r for r in events if r.get('event')=='ack'])==11163,'total acks')
    need(len([r for r in cases if r.get('case')=='full'])==688,'bounded full outcomes')
    for cycle in cycles:
        need([n(r,'count') for r in cases if r.get('case')=='full' and r.get('generation')==cycle.get('generation')]==list(range(0,n(cycle,'count')//16*16,16)),'exact full/channel outcomes')
    terminal=rows(raw,'[FL-M51-TERMINAL] ')
    need(len(terminal)==1 and terminal[0].get('exit')=='137' and terminal[0].get('notify')=='4' and terminal[0].get('late_reply')=='0','copy exit terminal')
    census=rows(raw,'[FL-CENSUS] ')
    need(len(census)==15 and all(all(r.get(k)=='0' for k in ('objects','views','frames')) for r in census),'zero mapping inventory')
    need(all(len({r.get(k) for r in census if 600<=n(r,'checkpoint')<=603})==1 for k in ('shm','tables')),'no cycle drift')
    bells=rows(raw,'[FL-BELL] ')
    registers=[r for r in bells if r.get('event')=='registered']
    completes=[r for r in bells if r.get('event')=='complete']
    takes=[r for r in bells if r.get('event')=='take']
    need(len(registers)==len(completes)==len(takes)==2,'two pending waits')
    for reg,comp,take in zip(registers,completes,takes):
        b=gens.get(reg.get('generation'),{})
        need(all(reg.get(k)==comp.get(k)==take.get(k)==b.get('consumer'+suffix) for k,suffix in [('consumer',''),('inc','_inc'),('vm','_vm')]),'waiter identity')
        need(reg.get('generation')==comp.get('generation')==take.get('generation') and comp.get('result')==take.get('result') and comp.get('sequence')==take.get('sequence'),'wait terminal collection')
    need(sorted(r.get('result','') for r in takes)==['0','9'],'wake/cancel outcomes')
    parks=rows(raw,'[FL-BELL-PARK] ')
    need(len(parks)==2 and all(all(r.get(k)==v for k,v in {'parked':'1','owner':'255','queued':'255','pins':'0','ready':'0'}.items()) for r in parks),'parked waiter proof')
    resources=rows(raw,'[FL-BELL-RESOURCE] ')
    need(resources and all(resources[-1].get(k)=='0' for k in ('active','waiters','receipts','pending')),'zero final channel/waiter inventory')
    maps=rows(raw,'[FL-MAP] ');frames=rows(raw,'[FL-FRAME] ');tlbs=rows(raw,'[FL-TLB] ')
    backs=[r for r in frames if r.get('event')=='backing'];objects=[]
    need(len(backs)==45, '45 physical backing objects required')
    need({(b.get('generation'),b.get('region')) for b in backs}=={(g,str(r)) for g in gens for r in range(3)}, 'backing/channel join')
    def seq(r): return n(r,'seq')
    for b in backs:
        same=lambda r: all(r.get(k)==b.get(k) for k in ('generation','region'))
        free=[r for r in frames if same(r) and r.get('event')=='free']
        ms=[r for r in maps if same(r) and r.get('event')=='map']
        need(len(free)==1 and free[0].get('frame')==b.get('frame'), 'exactly-once physical release')
        later=[r for r in backs if r.get('frame')==b.get('frame') and seq(r)>seq(b)]
        if free and later:
            need(seq(free[0])<min(seq(r) for r in later), 'physical backing reused before release')
        need(len(ms)==2 and {r.get('role') for r in ms}=={'0','1'}, 'exact two views')
        witnesses=[]
        for m in ms:
            binding=gens.get(m.get('generation'),{});role='producer' if m.get('role')=='0' else 'consumer'
            need(m.get('pid')==binding.get(role) and m.get('vm')==binding.get(role+'_vm') and m.get('inc')==binding.get(role+'_inc'), 'mapping role identity')
            writable=m.get('role')==('1' if m.get('region')=='1' else '0')
            need(m.get('write')==str(int(writable)) and m.get('nx')=='1' and m.get('frame')==b.get('frame'), 'mapping rights/backing')
            sameview=lambda r: all(r.get(k)==m.get(k) for k in ('generation','region','pid','inc','vm','va'))
            req=[r for r in maps if sameview(r) and r.get('event')=='request']
            done=[r for r in maps if sameview(r) and r.get('event')=='complete']
            ts=[r for r in tlbs if all(r.get(k)==m.get(k) for k in ('generation','region','vm','va'))]
            tr=[r for r in ts if r.get('event')=='request'];td=[r for r in ts if r.get('event')=='complete'];acks=[r for r in ts if r.get('event')=='ack']
            need(len(req)==len(done)==len(tr)==len(td)==1, 'missing unmap/TLB evidence')
            if len(req)==len(done)==len(tr)==len(td)==len(free)==1:
                need(seq(b)<seq(m)<seq(req[0])<seq(tr[0])<seq(td[0])<seq(done[0])<seq(free[0]), 'unmap before physical release ordering')
                mask=n(tr[0],'targets');targets={i for i in range(cpus) if mask&(1<<i)}
                need(mask>=0 and mask>>cpus==0 and {n(a,'cpu') for a in acks}==targets and len(acks)==len(targets), 'required remote ACK set')
                need(all(a.get('observed')==a.get('request')==tr[0].get('request') and a.get('targets')==tr[0].get('targets') and seq(tr[0])<seq(a)<seq(td[0]) for a in acks), 'ACK identity/order')
                need(td[0].get('request')==tr[0].get('request') and td[0].get('targets')==tr[0].get('targets'), 'TLB completion identity')
            witnesses.append({'mapping':m,'unmap_request':req,'unmap_complete':done,'tlb':ts})
        objects.append({'backing':b,'mappings':witnesses,'physical_free':free})
    need(len([r for r in maps if r.get('event')=='map'])==90, 'unaccounted mapping')
    remote=sum(r.get('event')=='request' and n(r,'targets')>0 for r in tlbs)
    zero=sum(r.get('event')=='request' and n(r,'targets')==0 for r in tlbs)
    need(remote>0 and zero>0, 'remote/zero target distinction')
    need(any(r.get('event')=='ack' and r.get('flushed')=='1' for r in tlbs), 'no actual remote flush')
    for group in (cases,bindings,events,copies,census,resources,maps,frames,tlbs):
        need(not any('_duplicate' in r for r in group),'duplicate field')
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'cases':cases,'copies':[r for r in copies if r.get('event')=='consumer'],'message_receipts':copies,
            'channels':len(gens),'publications':sum(r.get('event')=='publish' for r in events),
            'acknowledgements':sum(r.get('event')=='ack' for r in events),
            'mapping':{'objects':objects,'remote_unmaps':remote,'zero_target_unmaps':zero}}
