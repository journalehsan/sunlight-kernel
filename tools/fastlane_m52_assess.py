"""M5.2 native evidence: no synthetic transport or kernel publication receipts."""
from fastlane_channel_validate import rows
import re

def assess(raw, cpus, identity=None, failure=None, measurement=False):
    errors=[]
    def need(ok, why):
        if not ok: errors.append(why)
    def n(r,k):
        try: return int(r[k])
        except (KeyError,ValueError,TypeError): errors.append('missing integer '+k); return -1
    need(cpus in (2,4,12), 'topology')
    need(f'[SMP] Limine MP response: {cpus} CPU(s)' in raw and f'[SCHED] Per-core work-stealing scheduler: {cpus} online core(s)' in raw, 'online topology')
    need({int(x) for x in re.findall(r'\[SMP\] AP(\d+) .* online',raw)}==set(range(1,cpus)), 'online AP identities')
    need(identity is not None and all(identity.get(k) is True for k in ('source','elf','iso','services')), 'identity')
    need(failure is None, 'execution failure')
    need(not any(x in raw for x in ('PANIC','panicked at','checkpoint=FAIL','[FL-BELL-DEADLINE]')), 'guest failure')
    cases=rows(raw,'[FL-M52] ')
    bindings=[r for r in rows(raw,'[FL-BIND] ') if r.get('op')=='3' and r.get('state')=='3']
    gens={r.get('generation'):r for r in bindings}
    need(len(bindings)==len(gens)==27, '27 channels')
    expected={'start':[0], 'cycle':[1,16,1000,10000], 'slow-producer':[1], 'cancel':[0], 'malformed':[0,1,2,3], 'mutation':[16], 'revoke-copy-fault':[16], 'close-ack-fault':[16], 'consumer-exit-copy':[16], 'producer-exit':[16], 'complete':[0], 'duplicate-cursors':[17], 'invalid-cursor':[0,1], 'seeded-rollover':[(1<<32)-1], 'seeded-exhaustion':[(1<<64)-3], 'close-wait':[0], 'cancel-publication-fault':[0], 'registration-race':[1,2], 'close-notify-race':[3,4], 'rollback-empty':[1]}
    need(set(r.get('case') for r in cases)==set(expected)|{'full'}, 'case set')
    for case,counts in expected.items():
        need(sorted(n(r,'count') for r in cases if r.get('case')==case)==counts, 'case '+case)
    copies=rows(raw,'[FL-M52-MSG] ')
    activates=rows(raw,'[FL-NATIVE-RING] ')
    need(len(activates)==54, 'exact native role activation')
    assigned=[r for r in cases if r.get('case') not in ('start','complete','full')]
    need(len(assigned)==27 and {r.get('generation') for r in assigned}==set(gens), 'channel join')
    compact_generations={r.get('generation') for r in assigned if r.get('case')=='cycle'} if measurement else set()
    if measurement:
        from fastlane_m53_receipt import validate
        errors.extend(validate(raw,cpus))
    for case in assigned:
        g=case.get('generation'); b=gens.get(g,{})
        adversarial=case.get('case') in ('malformed','mutation','revoke-copy-fault','close-ack-fault','consumer-exit-copy','producer-exit')
        name=case.get('case'); start=0
        published=17 if adversarial else n(case,'count')
        consumed=(17 if name=='mutation' else 16) if adversarial else published
        if name=='rollback-empty': published=16; consumed=1
        if name=='invalid-cursor': published=consumed=16
        if name in ('seeded-rollover','seeded-exhaustion'): start=n(case,'count'); published=consumed=2
        if name=='registration-race': published=consumed=1
        if name=='close-notify-race': published=1; consumed=0
        need(b.get('producer')!=b.get('consumer') and b.get('producer_vm')!=b.get('consumer_vm'), 'independent peers')
        for role in ('producer','consumer'):
            need(all(n(b,k)>0 for k in (role,role+'_inc',role+'_vm')), 'authenticated identity')
            ms=[r for r in copies if r.get('generation')==g and r.get('event')==role]
            if g not in compact_generations:
                need([n(r,'message') for r in ms]==list(range(start,start+(published if role=='producer' else consumed))), role+' ordered exact count')
            else:
                need(not ms,'compact window emitted per-message log')
            for r in ms:
                need(r.get('pid')==b.get(role) and n(r,'bytes')==n(r,'message')%257 and n(r,'slot_generation')==n(r,'message')//16+1, 'message identity/length')
                if role=='consumer': need(r.get('coherent')==r.get('authentic')=='0', 'integrity')
            ac=[r for r in activates if r.get('generation')==g and r.get('role')==str(int(role=='consumer'))]
            need(len(ac)==1 and all(ac[0].get(k)==b.get(role+suffix) for k,suffix in [('pid',''),('inc','_inc'),('vm','_vm')]), 'activation binding')
        es=[r for r in copies if r.get('generation')==g]
        positions={(r.get('event'),n(r,'message')):i for i,r in enumerate(es)}
        for seq in range(start,start+(0 if g in compact_generations else consumed)):
            need(positions.get(('producer',seq),-1)<positions.get(('consumer',seq),-1), 'publish before copy')
            if seq>=start+16: need(positions.get(('consumer',seq-16),-1)<positions.get(('producer',seq),-1), 'reuse after private acknowledgement')
    need(sum(r.get('event')=='producer' for r in copies)==(227 if measurement else 11244), 'total publications')
    need(sum(r.get('event')=='consumer' for r in copies)==(202 if measurement else 11219), 'total snapshots/acks')
    full=[r for r in cases if r.get('case')=='full']
    need(len(full)==688, 'backpressure count')
    for case in cases:
        if case.get('case')=='cycle': need([n(r,'count') for r in full if r.get('generation')==case.get('generation')]==list(range(0,n(case,'count')//16*16,16)), 'full outcomes')
    # M5.1 private transitions must not be used as the M5.2 data plane.
    need(not rows(raw,'[FL-REPEAT] ') and not rows(raw,'[FL-TRANSFER] '), 'mediated transfer used')
    bells=rows(raw,'[FL-BELL] ')
    registers=[r for r in bells if r.get('event')=='registered']
    completes=[r for r in bells if r.get('event')=='complete']
    takes=[r for r in bells if r.get('event')=='take']
    need(len(registers)==len(completes)==len(takes)==6, 'six waits completed/collected')
    need(sorted(r.get('result','') for r in takes)==['0','0','0','0','4','9'], 'wake/cancel outcomes')
    for reg,comp,take in zip(registers,completes,takes):
        b=gens.get(reg.get('generation'),{})
        need(all(reg.get(k)==comp.get(k)==take.get(k)==b.get('consumer'+suffix) for k,suffix in [('consumer',''),('inc','_inc'),('vm','_vm')]), 'wait identity')
        need(reg.get('generation')==comp.get('generation')==take.get('generation') and comp.get('result')==take.get('result'), 'wait outcome')
    resources=rows(raw,'[FL-BELL-RESOURCE] ')
    need(resources and all(resources[-1].get(k)=='0' for k in ('active','waiters','receipts','pending')), 'final waiter inventory')
    census=rows(raw,'[FL-CENSUS] ')
    need(len(census)==27 and all(all(r.get(k)=='0' for k in ('objects','views','frames')) for r in census), 'zero restricted resources')
    need(all(len({r.get(k) for r in census if 600<=n(r,'checkpoint')<=603})==1 for k in ('shm','tables')), 'no cycle drift')
    faults=rows(raw,'[FL-PF] ')
    need(len(faults)==3 and raw.count('[FAULT] Page Fault at ')==3, 'three expected hardware faults only')
    for case,region,code in [('revoke-copy-fault','2',4),('close-ack-fault','1',6),('cancel-publication-fault','2',6)]:
        cs=[r for r in cases if r.get('case')==case]
        if len(cs)!=1: continue
        g=cs[0].get('generation'); b=gens.get(g,{})
        role='producer' if case=='cancel-publication-fault' else 'consumer'
        fs=[r for r in faults if r.get('pid')==b.get(role) and r.get('vm')==b.get(role+'_vm')]
        need(len(fs)==1 and n(fs[0],'error')==code, 'expected fault identity/bits')
        maps_for=[r for r in rows(raw,'[FL-MAP] ') if r.get('event')=='map' and r.get('generation')==g and r.get('role')==str(int(role=='consumer')) and r.get('region')==region]
        need(len(maps_for)==1 and len(fs)==1 and n(maps_for[0],'va')<=n(fs[0],'va')<n(maps_for[0],'va')+4096, 'fault in withdrawn view')
    races=rows(raw,'[FL-BELL-RACE] ')
    for mode in range(1,5):
        rs=[r for r in races if r.get('mode')==str(mode)]
        need(len([r for r in rs if r.get('event')=='armed'])==1, 'race arm')
        need(any(r.get('event')=='lock-contended' for r in rs), 'race actual contention')
        need({r.get('actor') for r in rs if r.get('event')=='returned'}=={'1','2'}, 'race finished actors')
    diagnostics=rows(raw,'[FL-M52-KDIAG] ')
    cleanup=rows(raw,'[FL-M52-CLEAN] ')
    need(len(diagnostics)==len(cleanup)==25, 'bounded diagnostic samples')
    need(all(n(r,'context_switches')>0 and n(r,'syscalls')>0 for r in diagnostics), 'actual diagnostic counters')
    need([n(r,'word_calls') for r in diagnostics[:4]]==[2,4,6,8], 'no per-message word syscalls')
    need(all(n(r,'data_plane_allocations')==0 and n(r,'backing_allocations')==3 and 0<=n(r,'duration_ms')<5000 for r in cleanup), 'cleanup/allocation observations')
    maps=rows(raw,'[FL-MAP] ');frames=rows(raw,'[FL-FRAME] ');tlbs=rows(raw,'[FL-TLB] ')
    backs=[r for r in frames if r.get('event')=='backing'];objects=[]
    need(len(backs)==81, '81 physical backing objects required')
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
    need(len([r for r in maps if r.get('event')=='map'])==162, 'unaccounted mapping')
    remote=sum(r.get('event')=='request' and n(r,'targets')>0 for r in tlbs)
    zero=sum(r.get('event')=='request' and n(r,'targets')==0 for r in tlbs)
    need(remote>0 and zero>0, 'remote/zero target distinction')
    need(any(r.get('event')=='ack' and r.get('flushed')=='1' for r in tlbs), 'no actual remote flush')
    for group in (cases,bindings,copies,activates,census,resources,maps,frames,tlbs,faults,diagnostics,cleanup):
        need(not any('_duplicate' in r for r in group), 'duplicate trace field')
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'cases':cases,'copies':[r for r in copies if r.get('event')=='consumer'],'message_receipts':copies, 'channels':len(gens), 'mapping':{'objects':objects,'remote_unmaps':remote,'zero_target_unmaps':zero}, 'diagnostics':diagnostics, 'cleanup':cleanup, 'faults':faults}
