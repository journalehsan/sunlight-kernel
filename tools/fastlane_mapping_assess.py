"""Reject missing native evidence. Synthetic parser controls are never native proof."""
import re
from fastlane_channel_validate import rows

def assess_mapping(raw, cpus):
    errors=[]
    def need(ok, why):
        if not ok: errors.append(why)
    users=rows(raw,'[FL-M2] ')
    maps=rows(raw,'[FL-MAP] ')
    frames=rows(raw,'[FL-FRAME] ')
    tlbs=rows(raw,'[FL-TLB] ')
    faults=rows(raw,'[FL-PF] ')
    accesses=rows(raw,'[FL-ACCESS] ')
    census=rows(raw,'[FL-CENSUS] ')
    rollbacks=rows(raw,'[FL-ROLLBACK] ')
    bindings={r['generation']:r for r in rows(raw,'[FL-BIND] ') if r.get('op')=='2'}
    need(not any(x in raw for x in ('PANIC','panicked at','checkpoint=FAIL')), 'unexpected guest assertion')
    need(not any('_duplicate' in r for r in users+maps+frames+tlbs+faults+accesses+census+rollbacks),'duplicate trace field')
    def n(r,k):
        try:return int(r[k])
        except (KeyError,ValueError,TypeError):errors.append('missing/invalid integer '+k);return -1
    def seq(r):return n(r,'seq')
    for case,count in {'start':1,'authorized-regions':2,'alias-escalation-denied':2,'protected-write-fault':2,
                       'producer-state-write-fault':1,'revoked-access-fault':2,'mapping-rollback':1,
                       'exec-retirement':1,'remote-revocation-fault':1,'lifecycle-cycle':6,'complete':1}.items():
        need(sum(r.get('case')==case for r in users)==count, 'missing/duplicate user case '+case)
    need({r.get('detail') for r in users if r.get('case')=='lifecycle-cycle'}==set(map(str,range(6))),'repeated cycle coverage')
    need(len(faults)==len(accesses)==6,'six disposable hardware fault witnesses required')
    for a in accesses:
        selected=[f for f in faults if f.get('pid')==a.get('pid') and f.get('va')==a.get('va')]
        need(len(selected)==1,'fault not correlated to exact attempted VA/PID')
        need(any(u.get('generation')==a.get('generation') and u.get('detail')==a.get('pid') and u.get('case','').endswith('fault') for u in users),'fault lacks independent waitpid receipt')
        if len(selected)==1:
            f=selected[0]; write=n(a,'write'); code=n(f,'error')
            need(code & 4 != 0 and bool(code & 2)==bool(write), 'fault is not the expected user read/write access')
            selected_maps=[m for m in maps if m.get('event')=='map' and all(m.get(k)==a.get(k) for k in ('generation','pid','va'))]
            need(len(selected_maps)==1,'fault lacks original exact mapping')
            if len(selected_maps)==1:
                m=selected_maps[0]
                need(all(f.get(k)==m.get(k) for k in ('pid','inc','vm','va')),'fault recipient/VM mismatch')
                stale=a.get('expected')=='remote-stale' or any(u.get('case')=='revoked-access-fault' and u.get('detail')==a.get('pid') for u in users)
                need(code==(6 if write else 4) if stale else code==7,'wrong hardware present/protection error bits')
    # try_current_pid is intentionally nonblocking and may report u64::MAX
    # while another CPU holds SCHEDULER. The later locked FL-PF record is the
    # authoritative identity. Still require one early diagnostic per real fault.
    diagnostics=re.findall(r'\[FAULT\] #14 pid=(\d+).*?err=0x([0-9a-f]+)',raw)
    need(len(diagnostics)==len(faults),'unexpected or missing page fault')
    for (pid,code),f in zip(diagnostics,faults):
        need(pid in (f.get('pid'),str(2**64-1)) and int(code,16)==n(f,'error'),'early fault diagnostic differs from locked fault witness')
    need(raw.count('[FAULT] Page Fault at ')==len(faults),'unaccounted unrecoverable page fault')
    need(not any(x in raw for x in ('[FAULT] #0 ','[FAULT] #6 ','[FAULT] #13 ')), 'unexpected non-page access fault')
    need(len(rollbacks)==1 and all(n(r,'free_before')==n(r,'free_after') and r.get('published')=='0' for r in rollbacks),'rollback did not reconcile real allocator')
    need(len(census)>=14,'missing bounded resource census')
    for c in census:need(all(n(c,k)==0 for k in ('objects','views','frames')),'restricted resource drift')
    cycles=[c for c in census if 100<=n(c,'checkpoint')<=105]
    need(len(cycles)==6 and all(len({c.get(k) for c in cycles})==1 for k in ('shm','tables')),'repeated cycles changed shared frame total')
    # Channel-specific retirement inventory. Each object owns exactly one frame;
    # each mapping must fully unmap before its last frame becomes PMM Free.
    backs=[r for r in frames if r.get('event')=='backing'];objects=[]
    need(len(backs)>=30,'missing mapping workload/backing evidence')
    all_sequences=[seq(r) for r in maps+frames+tlbs]
    need(len(all_sequences)==len(set(all_sequences)),'duplicate retirement sequence')
    for b in backs:
        g=b.get('generation');region=b.get('region');frame=b.get('frame')
        selected=[m for m in maps if m.get('event')=='map' and m.get('generation')==g and m.get('region')==region]
        free=[f for f in frames if f.get('event')=='free' and all(f.get(k)==b.get(k) for k in ('generation','region','frame'))]
        need(len(free)==1,'missing/duplicate final physical free')
        need(1<=len(selected)<=2,'missing/excess role mappings')
        need(len({m.get('role') for m in selected})==len(selected),'duplicate role view')
        evidence=[]
        for m in selected:
            need(m.get('frame')==frame and seq(m)>seq(b),'wrong backing frame or publication order')
            role=n(m,'role');reg=n(m,'region');binding=bindings.get(g,{})
            prefix='producer' if role==0 else 'consumer'
            need(role in (0,1) and reg in (0,1,2),'invalid mapping role/region')
            need(all(m.get(k)==binding.get(prefix+suffix) for k,suffix in (('pid',''),('inc','_inc'),('vm','_vm'))),'mapping lacks authenticated exact role binding')
            need(n(m,'write')==int(role==1 if reg==1 else role==0) and m.get('nx')=='1','writable role escalation or executable mapping')
            key=('generation','region','pid','inc','vm','va')
            req=[r for r in maps if r.get('event')=='request' and all(r.get(k)==m.get(k) for k in key)]
            done=[r for r in maps if r.get('event')=='complete' and all(r.get(k)==m.get(k) for k in key)]
            need(len(req)==len(done)==1,'mapping lacks exact unmap request/completion')
            ts=[r for r in tlbs if all(r.get(k)==m.get(k) for k in ('generation','region','vm','va'))]
            tr=[r for r in ts if r.get('event')=='request'];td=[r for r in ts if r.get('event')=='complete'];acks=[r for r in ts if r.get('event')=='ack']
            need(len(tr)==len(td)==1,'missing synchronous TLB completion')
            if len(req)==len(done)==len(tr)==len(td)==1:
                need(seq(m)<seq(req[0])<seq(tr[0])<seq(td[0])<seq(done[0]),'unmap/TLB event order')
                if len(free)==1:need(seq(done[0])<seq(free[0]),'frame freed before unmap and ACK completion')
                mask=n(tr[0],'targets');target_cpus={i for i in range(cpus) if mask&(1<<i)}
                need(mask>=0 and mask>>cpus==0,'invalid remote target mask')
                need({n(a,'cpu') for a in acks}==target_cpus and len(acks)==len(target_cpus),'remote acknowledgements missing/duplicate')
                need(all(a.get('observed')==tr[0].get('request') and a.get('request')==tr[0].get('request') and a.get('targets')==tr[0].get('targets') and seq(tr[0])<seq(a)<seq(td[0]) for a in acks),'ACK does not match actual handler request')
                need(td[0].get('request')==tr[0].get('request') and td[0].get('targets')==tr[0].get('targets'),'TLB completion changed identity')
            evidence.append({'mapping':m,'unmap_request':req,'unmap_complete':done,'tlb':ts})
        objects.append({'backing':b,'mappings':evidence,'physical_free':free})
    need(len([m for m in maps if m.get('event')=='map'])==sum(len(o['mappings']) for o in objects),'unaccounted mapping')
    remote_generations={u.get('generation') for u in users if u.get('case')=='remote-revocation-fault'}
    need(any(t.get('generation') in remote_generations and t.get('event')=='ack' and t.get('flushed')=='1' for t in tlbs),'required still-running remote VM was not flushed')
    remote=sum(t.get('event')=='request' and n(t,'targets')>0 for t in tlbs)
    zero=sum(t.get('event')=='request' and n(t,'targets')==0 for t in tlbs)
    need(remote>0 and zero>0,'remote and zero-target cases must be distinguished and exercised')
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'objects':objects,'faults':faults,'census':census,'rollbacks':rollbacks,'remote_unmaps':remote,'zero_target_unmaps':zero}
