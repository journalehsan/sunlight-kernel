"""M6 receipts: complete population, independently qualified local pairs, exact retirement."""
from collections import Counter
from math import ceil
from fastlane_channel_validate import rows
CONFIGS = ((1,64),(2,64),(1,256),(2,256))
COUNT = 128

def qualify(t, c):
    """Return a rejection reason or the qualified duration. Never trust guest verdict alone."""
    try:
        n = {k:int(v) for k,v in t.items() if k in ('a','b','ca','cb','ea','eb','ppm','ns')}
        v = {k:int(c[k]) for k in ('ha','hb','pa','pb','fa','fb','sa','sb')}
        if n['ca'] != 0 or n['cb'] != 0: return 'nonresident'
        if not n['ea'] or not n['eb']: return 'uncalibrated'
        if n['ea'] != n['eb']: return 'stale-epoch'
        if n['b'] < n['a']: return 'backward'
        if any(v[x] != v[y] for x,y in (('ha','hb'),('pa','pb'),('fa','fb'),('sa','sb'))): return 'calibration-change'
        if not v['fa'] & 32 or not v['fa'] & 4: return 'unstable'
        if not v['ha'] or not 0 < v['pa'] <= 20000 or v['pa'] != n['ppm']: return 'uncertainty'
        duration = n['b'] - n['a']
        if duration <= 0: return 'zero'
        if t['why'] != 'accepted' or n['ns'] != duration: return 'guest-host-disagreement'
        return duration
    except (ValueError,KeyError,TypeError): return 'malformed'

def retirement(raw, cpus):
    """Same generation/region and authenticated view contract as R2, for this fixture."""
    errors=[]
    def need(ok, why):
        if not ok: errors.append(why)
    maps=rows(raw,'[FL-MAP] '); frames=rows(raw,'[FL-FRAME] '); tlbs=rows(raw,'[FL-TLB] ')
    binds=rows(raw,'[FL-BIND] '); binds=[b for b in binds if b.get('op')=='2']
    binding={b['generation']:b for b in binds}
    need(len(binding)==len(binds),'duplicate binding')
    seq=lambda r:int(r['seq'])
    backs=[r for r in frames if r['event']=='backing']; frees=[r for r in frames if r['event']=='free']
    life=lambda r:(r['generation'],r['region'])
    need(len(backs)==12,'backing inventory') # four transfer channels, three pages each
    need(len({life(r) for r in backs})==len(backs),'duplicate lifetime')
    need(len(frees)==len(backs),'physical retirement inventory')
    need(len(maps)==len(backs)*6,'exact two-view mapping inventory')
    need(len({seq(r) for r in maps+frames+tlbs})==len(maps+frames+tlbs),'duplicate retirement sequence')
    attributed_maps=[];attributed_tlbs=[];attributed_frees=[]
    remote=0
    for b in backs:
        g,r=life(b); need(int(g)>0 and int(r) in (0,1,2) and int(b['frame'])>0,'backing identity')
        fs=[f for f in frees if life(f)==life(b) and f['frame']==b['frame']]
        need(len(fs)==1,'exactly once free'); attributed_frees+=fs
        views=[m for m in maps if m['event']=='map' and life(m)==life(b)]
        need(len(views)==2 and {m['role'] for m in views}=={'0','1'},'exact role views')
        for m in views:
            binding_row=binding.get(g,{})
            prefix='producer' if m['role']=='0' else 'consumer'
            need(all(m[k]==binding_row.get(prefix+suffix) for k,suffix in (('pid',''),('inc','_inc'),('vm','_vm'))),'authenticated view identity')
            need(m['nx']=='1' and int(m['write'])==int(m['role']=='1' if r=='1' else m['role']=='0'),'restricted permissions')
            need(m['frame']==b['frame'] and seq(b)<seq(m),'backing publication')
            key=('generation','region','pid','inc','vm','va')
            chosen=[v for v in maps if all(v[k]==m[k] for k in key)];attributed_maps+=chosen
            req=[v for v in chosen if v['event']=='request'];done=[v for v in chosen if v['event']=='complete']
            ts=[v for v in tlbs if all(v[k]==m[k] for k in ('generation','region','vm','va'))];attributed_tlbs+=ts
            tr=[v for v in ts if v['event']=='request'];td=[v for v in ts if v['event']=='complete'];acks=[v for v in ts if v['event']=='ack']
            need(len(chosen)==3 and len(req)==len(done)==len(tr)==len(td)==len(fs)==1,'complete retirement chain')
            if len(req)==len(done)==len(tr)==len(td)==len(fs)==1:
                need(seq(m)<seq(req[0])<seq(tr[0])<seq(td[0])<seq(done[0])<seq(fs[0]),'ACK before unmap before free')
                mask=int(tr[0]['targets']);cpu=int(tr[0]['cpu']);targets={i for i in range(cpus) if mask&(1<<i)}
                remote+=bool(mask)
                need(0<=cpu<cpus and 0<=mask<1<<cpus and not mask&(1<<cpu),'target set bounds')
                need(len(acks)==len(targets) and {int(a['cpu']) for a in acks}==targets,'exact remote ACK set')
                need(all(v['cpu']==tr[0]['cpu'] for v in (req[0],td[0],done[0])),'initiator identity')
                need(td[0]['request']==tr[0]['request'] and td[0]['targets']==tr[0]['targets'],'request completion identity')
                need(all(a['observed']==a['request']==tr[0]['request'] and a['targets']==tr[0]['targets'] and a['flushed']=='1' and seq(tr[0])<seq(a)<seq(td[0]) for a in acks),'ACK request ordering')
    need(len(attributed_maps)==len(maps) and len(attributed_tlbs)==len(tlbs) and len(attributed_frees)==len(frees),'orphan retirement event')
    censuses=rows(raw,'[FL-CENSUS] ')
    need({r['checkpoint'] for r in censuses}=={'864','964','1056','1156','999'} and len(censuses)==5,'census coverage')
    need(all(all(int(r[k])==0 for k in ('objects','views','frames')) for r in censuses),'scoped inventory')
    active=set()
    for line in raw.splitlines():
        if line.startswith('[FL-FRAME] '):
            r=rows(line,'[FL-FRAME] ')[0]
            if r['event']=='backing':active.add(life(r))
            if r['event']=='free':active.discard(life(r))
        if line.startswith('[FL-CENSUS] '):need(not active,'live fixture at census')
    need(not active,'live fixture at end')
    return {'status':'FAIL' if errors else 'PASS','errors':sorted(set(errors)),'backings':len(backs),'physical_frees':len(frees),'remote_requests':remote,'census':censuses,'cross_cpu_remote_positive_proof':'REUSED_M54R2'}

def assess(raw,cpus):
    errors=[];result=[]
    if len(raw)>32*1024*1024: return {'status':'FAIL','errors':['receipt too large'],'results':[]}
    try:
        ts=rows(raw,'[FL-M60-T] ');cs=rows(raw,'[FL-M60-C] ');ps=rows(raw,'[FL-M60-P] ');rs=rows(raw,'[FL-M60-R] ')
        if any('_duplicate' in r for r in ts+cs+ps+rs):errors.append('duplicate field')
        if any(m in raw for m in ('PANIC','panicked at','checkpoint=FAIL','[FAULT]')):errors.append('guest failure')
        if raw.count('[FL-M60] case=complete configurations=4 resources=0 integrity=exact historical=UNRESOLVED production=DISABLED')!=1:errors.append('incomplete terminal receipt')
        if len(ps)!=4 or len(rs)!=4:errors.append('actor receipt count')
        key=lambda r:(r['k'],r['p'],r['z'],r['s'],r['pid'])
        clock={key(r):r for r in cs}
        if len(clock)!=len(cs) or len(ts)!=len(cs) or {key(r) for r in ts}!=set(clock) or len({key(r) for r in ts})!=len(ts):errors.append('missing/duplicate clock metadata')
        for path,size in CONFIGS:
            select=lambda rows:[r for r in rows if (int(r['p']),int(r['z']))==(path,size)]
            p=select(ps);r=select(rs)
            if len(p)!=1 or len(r)!=1:errors.append('configuration receipts');continue
            p=p[0];r=r[0]
            if [int(p[k]) for k in ('warm','measured','sent','replies','cpu','outstanding','retries')]!=[16,128,144,144,0,1,0] or [int(r[k]) for k in ('received','verified','acks')]!=[144]*3 or p['producer']!=r['peer'] or p['consumer']!=r['pid']:errors.append('counter/identity reconciliation')
            observations=[];groups={}
            for k in ('setup','window','teardown','rtt','consume')+(() if path==0 else ('publish-notify',)):
                selected=[t for t in select(ts) if t['k']==k]
                expect=COUNT if k in ('rtt','consume','publish-notify') else 1
                if len(selected)!=expect or {int(t['s']) for t in selected}!=set(range(expect)):errors.append('sample population '+k)
                accepted=[];rejected=Counter()
                for t in selected:
                    c=clock.get(key(t),{});q=qualify(t,c)
                    expected_pid=r['pid'] if k=='consume' else p['producer']
                    if t['pid']!=expected_pid:errors.append('observation actor identity')
                    item={'timestamp':t,'calibration':c,'qualification':'ACCEPTED' if isinstance(q,int) else 'REJECTED','reason':None if isinstance(q,int) else q}
                    if isinstance(q,int):
                        item['duration_ns']=q;item['scale_uncertainty_ns']=ceil(q*int(c['pa'])/1_000_000)+1;accepted.append(q)
                    else:rejected[q]+=1
                    observations.append(item)
                accepted.sort()
                g={'accepted':len(accepted),'rejected':dict(rejected),'status':'QUALIFIED' if len(accepted)>=(100 if expect==COUNT else 1) else 'NOT_QUALIFIED'}
                if expect==1:g['duration_ns']=accepted[0] if accepted else None
                elif g['status']=='QUALIFIED':g['percentiles_ns']={str(pc):accepted[ceil(len(accepted)*pc/100)-1] for pc in (50,95,99)}
                groups[k]=g
            duration=groups['window'].get('duration_ns')
            groups['window']['messages_per_second']=COUNT*1e9/duration if duration else None
            groups['window']['payload_MiB_per_second']=COUNT*size*1e9/(duration*1048576) if duration else None
            result.append({'path':path,'payload_bytes':size,'measured_messages':COUNT,'warmup_messages':16,'metric':'verified-request-ack','producer_cpu':0,'consumer_cpu':0,'metrics':groups,'observations':observations})
        if len(ts)!=(4*(3+128*3)):errors.append('unexpected observation inventory')
        kcpu=rows(raw,'[FL-M54-KCPU] ');actors=rows(raw,'[FL-M53-KACTOR] ')
        if len(kcpu)!=4 or len(actors)!=4:errors.append('CPU receipt inventory')
        for c in kcpu:
            if c['charged_valid']!='1' or c['discontinuities']!='0' or c['wall_valid']!='1' or c['migrated']!='0' or c['cpu_start']!=c['cpu_stop'] or c['cpu_stop']!='0' or c['reconcile']!='consistent' or c['wall_contract']!='same-cpu-only' or int(c['charged_ns'])>int(c['wall_ns']):errors.append('CPU reconciliation')
        for a in actors:
            if a['overflow']!='0' or a['dropped']!='0' or a['runtime_delta_valid']!='1':errors.append('CPU counter overflow')
        retired=retirement(raw,cpus);errors+=retired['errors']
        qualified=len(result)==4 and all(all(m['status']=='QUALIFIED' for m in r['metrics'].values()) for r in result)
        return {'status':'FAIL' if errors else 'PASS' if qualified else 'NOT_QUALIFIED','errors':sorted(set(errors)),'results':result,'retirement':retired,'cpu_time':kcpu,'actor_counters':actors,'cross_cpu_latency':'NOT_QUALIFIED','production_fastlane':'DISABLED'}
    except (ValueError,KeyError,TypeError,IndexError) as e:
        return {'status':'FAIL','errors':errors+['malformed receipt: '+str(e)],'results':result}
