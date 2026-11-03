"""Bounded M5.3 correctness receipts. Never calculate performance comparisons."""
import hashlib
import json
from fastlane_channel_validate import rows
MAX_RECEIPT_BYTES = 65536

def validate(raw, cpus):
    errors=[]
    def need(ok,why):
        if not ok: errors.append('M53 '+why)
    def num(r,k):
        try:return int(r[k])
        except (KeyError,ValueError,TypeError):errors.append('M53 missing integer '+k);return -1
    actors=rows(raw,'[FL-M53-ACTOR] ')
    cycles=[r for r in rows(raw,'[FL-M52] ') if r.get('case')=='cycle']
    bindings={r.get('generation'):r for r in rows(raw,'[FL-BIND] ') if r.get('op')=='3' and r.get('state')=='3'}
    setup=rows(raw,'[FL-M53-SETUP] ')
    sample_rows=rows(raw,'[FL-M53-SAMPLES] ')
    def samples_for(actor):
        found=[r for r in sample_rows if all(r.get(k)==actor.get(k) for k in ('generation','pid'))]
        need([num(r,'first') for r in found]==list(range(0,min(num(actor,'count'),64),4)),'sample chunk order and inventory')
        try:return [int(t) for r in found for t in r.get('times','').split(',')]
        except ValueError:return []
    need(len(actors)==8 and len(cycles)==4 and len(setup)==4,'bounded exact receipt count')
    for cycle in cycles:
        g=cycle.get('generation');count=num(cycle,'count');warmup=16 if count>16 else 0
        ar=[r for r in actors if r.get('generation')==g]
        b=bindings.get(g,{})
        need(len(ar)==2 and {r.get('pid') for r in ar}=={b.get('producer'),b.get('consumer')},'actor identity')
        for r in ar:
            need(num(r,'start_sequence')==warmup and num(r,'next')==count and num(r,'count')==count-warmup,'count and warmup')
            need(num(r,'bytes')==sum(i%257 for i in range(warmup,count)),'payload byte count')
            need(num(r,'full')==(count//16-warmup//16 if r.get('pid')==b.get('producer') else 0),'backpressure count')
            need(all(r.get(k)==v for k,v in {'overflow':'0','order':'1','coherent':'0','authentic':'0','clock':'kernel-bsp-ms','resolution_ms':'10'}.items()),'counter/integrity contract')
            samples=samples_for(r)
            need(num(r,'sample_count')==len(samples),'declared sample count')
            need(len(samples)==min(count-warmup,64) and num(r,'dropped')==max(count-warmup-64,0),'bounded sample inventory')
            start=num(r,'start_ms');end=num(r,'end_ms')
            need(samples==sorted(samples) and all(start<=t<=end for t in samples),'sample clock interval')
            need(0<=start<=end<184467440737095516,'nondecreasing unsaturated coarse clock')
        if len(ar)==2:
            producer=next((r for r in ar if r.get('pid')==b.get('producer')), {})
            consumer=next((r for r in ar if r.get('pid')==b.get('consumer')), {})
            try:
                ps=samples_for(producer)
                cs=samples_for(consumer)
                need(len(ps)==len(cs) and all(p<=c for p,c in zip(ps,cs)),'publication-start before validated receipt')
            except ValueError:need(False,'sample parse')
        st=[r for r in setup if r.get('generation')==g]
        need(len(st)==1,'setup join')
        if len(st)==1:
            need(0<=num(st[0],'start_ms')<=num(st[0],'end_ms')<=min((num(r,'start_ms') for r in ar),default=-1),'setup excluded')
    kernel=rows(raw,'[FL-M53-KACTOR] ')
    need(len(kernel)==8,'exact per-actor kernel counters')
    for a in actors:
        ks=[k for k in kernel if all(k.get(f)==a.get(f) for f in ('generation','pid'))]
        need(len(ks)==1,'kernel actor join')
        if len(ks)!=1:continue
        k=ks[0];b=bindings.get(a.get('generation'),{})
        role='producer' if a.get('pid')==b.get('producer') else 'consumer'
        need(k.get('inc')==b.get(role+'_inc') and k.get('vm')==b.get(role+'_vm'),'kernel incarnation')
        need(num(k,'notifications')==(2*((num(a,'count')+15)//16) if role=='producer' else 0),'notification count')
        need(num(k,'syscalls')>0 and num(k,'context_switch_ins')>=0 and num(k,'wake_publications')>=0 and k.get('overflow')==k.get('dropped')=='0','kernel bounded counters')
        need(num(k,'start_ms')<=num(a,'start_ms')<=num(a,'end_ms')<=num(k,'end_ms'),'nested actor window')
        need(num(k,'cpu')==int(role=='consumer'),'restored actor placement')
    probes=rows(raw,'[FL-M53-CLOCK] ')
    need(len(probes)==16,'bounded migration clock probes')
    for p in probes:
        need(num(p,'cpu') in (0,1) and num(p,'cpu')<cpus and num(p,'reads')==257 and num(p,'regressions')==0 and 0<=num(p,'begin_ms')<=num(p,'end_ms'),'clock probe')
    for pid in {p.get('pid') for p in probes}:
        ps=[p for p in probes if p.get('pid')==pid]
        need({p.get('cpu') for p in ps}=={'0','1'},'migration observed by affinity calls')
        need(all(num(a,'end_ms')<=num(b,'begin_ms') for a,b in zip(ps,ps[1:])),'cross-migration monotonicity')
    need(len(sample_rows)==sum((min(num(a,'count'),64)+3)//4 for a in actors),'bounded total sample rows')
    need(all(any(all(s.get(k)==a.get(k) for k in ('generation','pid')) for a in actors) for s in sample_rows),'no orphan sample rows')
    for group in (actors,setup,probes,kernel,sample_rows):
        need(not any('_duplicate' in r for r in group),'duplicate field')
    return errors

def receipt(raw,cpus,image,source,assessment):
    actors=rows(raw,'[FL-M53-ACTOR] ')
    out={'schema':'sunlight.fastlane.measurement-readiness.v1','milestone':'M5.3',
         'status':assessment['status'],'errors':assessment['errors'],
         'source_identity':hashlib.sha256(json.dumps(source,sort_keys=True).encode()).hexdigest(),
         'image_identity':{k:image[k] for k in ('elf_sha256','iso_sha256')},
         'cpu_topology':{'online':cpus,'producer_affinity':0,'consumer_affinity':1,'clock_probe_cpus':[0,1]},
         'workload':{'transport':'shared-ring','total_counts':[1,16,1000,10000],'warmup_counts':[0,0,16,16],
                     'measurement_counts':[1,16,984,9984],'capacity':16,'payload_max':256,'duration_mode':'fixed-count'},
         'actors':actors,'latency_samples':rows(raw,'[FL-M53-SAMPLES] '),'kernel_actors':rows(raw,'[FL-M53-KACTOR] '),'setup':rows(raw,'[FL-M53-SETUP] '),'retirement':assessment.get('cleanup',[])[:4],
         'clock_probes':rows(raw,'[FL-M53-CLOCK] '),
         'timing':{'source':'kernel-bsp-ms','resolution_ms':10,'valid_for_elapsed_windows':not validate(raw,cpus),
                   'valid_for_per_message_percentiles':False,'overhead':'257 syscall clock reads per probe plus first 64 message timestamps per measured actor window; no subtraction',
                   'zero_duration':'INVALID_FOR_RATES','tsc_cross_cpu_validation':'NOT_CLAIMED'},
         'unavailable_metrics':['qualified-per-message-latency','qualified-p50','qualified-p95','qualified-p99','per-actor-cpu-time','kernel-user-time-split','measured-allocation-count','lock-contention-time'],
         'system_diagnostics':assessment.get('diagnostics',[])[:4],
         'expected_adversarial_outcomes':{'hardware_faults':3,'coherent_publication':'NOT PROVIDED','application_authenticity':'NOT PROVIDED'},
         'integrity':{'cooperative_payload':'exact expected bytes asserted before acknowledgement','hostile_peer':'private snapshot only',
                      'memory_safety':'PROVIDED','stable_private_snapshots':'PROVIDED','coherent_publication':'NOT PROVIDED','application_authenticity':'NOT PROVIDED'},
         'final_inventory':{'restricted':rows(raw,'[FL-CENSUS] ')[-1:], 'doorbell':rows(raw,'[FL-BELL-RESOURCE] ')[-1:]},
         'm6_decision':'NOT_READY: coarse clock and unavailable actor metrics; no comparative benchmarks run'}
    assert len(json.dumps(out).encode())<=MAX_RECEIPT_BYTES,'receipt size bound'
    return out
