"""M5 source-bound native acceptance. Synthetic parser checks are separate."""
from fastlane_channel_validate import rows


def assess(raw, cpus, identity=None, failure=None):
    errors = []
    def need(ok, why):
        if not ok: errors.append(why)
    def n(r, k):
        try: return int(r[k])
        except (KeyError, ValueError, TypeError):
            errors.append('missing integer: ' + k); return -1
    need(cpus in (2, 4, 12), 'unsupported topology')
    need(identity is not None and all(identity.get(k) is True for k in ('source','elf','iso','services')), 'identity missing')
    need(failure is None, 'execution failure')
    need(not any(s in raw for s in ('PANIC','panicked at','checkpoint=FAIL','[FL-BELL-DEADLINE]','[FL-RACE-DEADLINE]')), 'guest assertion')
    cases = rows(raw, '[FL-M5] ')
    expected = {'start':[0], 'cycle':list(range(8)), 'malformed':list(range(4)),
                'mutation-during-copy':[0], 'close-before-ack':[0], 'revoke-during-copy':[0],
                'cancel-wait':[0], 'producer-exit':[0,1], 'consumer-exit-before-copy':[0],
                'remote-retirement':[0], 'complete':[0]}
    need(set(r.get('case') for r in cases) == set(expected), 'case set')
    for case, details in expected.items():
        need(sorted(r.get('detail','') for r in cases if r.get('case')==case) == sorted(map(str,details)), 'case count '+case)
    bindings = [r for r in rows(raw,'[FL-BIND] ') if r.get('op')=='3' and r.get('state')=='3']
    need(len(bindings)==20, '20 authenticated pairs required')
    gens = {r.get('generation'):r for r in bindings}
    need(len(gens)==20, 'generation reuse')
    for r in bindings:
        need(r.get('producer')!=r.get('consumer') and r.get('producer_vm')!=r.get('consumer_vm'), 'participants not independent')
        need(all(n(r,k)>0 for k in ('issuer_inc','epoch','producer_inc','consumer_inc','producer_vm','consumer_vm')), 'unauthenticated role')
    assigned=[r.get('generation') for r in cases if r.get('case') not in ('start','complete')]
    need(len(assigned)==len(set(assigned))==20 and set(assigned)==set(gens), 'case/channel join')
    success={r.get('generation') for r in cases if r.get('case') in ('cycle','mutation-during-copy','remote-retirement')}
    excluded={r.get('generation') for r in cases if r.get('case')=='cancel-wait' or (r.get('case')=='producer-exit' and r.get('detail')=='0')}
    copied=success|{r.get('generation') for r in cases if r.get('case')=='close-before-ack'}
    pubs = rows(raw,'[FL-TRANSFER] ')
    need({r.get('generation') for r in pubs if r.get('event')=='publish'}==set(gens)-excluded, 'publication/case join')
    need({r.get('generation') for r in pubs if r.get('event')=='ack'}==success, 'acknowledgement/case join')
    need(sum(r.get('event')=='publish' for r in pubs)==18, 'publication count')
    need(sum(r.get('event')=='ack' for r in pubs)==10, 'ack count')
    copies = rows(raw,'[FL-M5-COPY] ')
    need(len(copies)==11 and sum(r.get('mixed')=='1' for r in copies)==1, 'private-copy coverage')
    need({r.get('generation') for r in copies}==copied, 'copy/case join')
    for r in copies:
        b=gens.get(r.get('generation'),{})
        need(r.get('pid')==b.get('consumer') and all(r.get(k)==v for k,v in {'message':'0','slot_generation':'1','bytes':'256','stable':'1','coherent':'0','authentic':'0'}.items()), 'invalid private copy')
    for g,b in gens.items():
        events=[r for r in pubs if r.get('generation')==g]
        p=[r for r in events if r.get('event')=='publish'];a=[r for r in events if r.get('event')=='ack']
        need(len(p)<=1 and len(a)<=1 and (not a or len(p)==1), 'duplicate or unowned publication/ack')
        for r in events:
            role='producer' if r.get('event')=='publish' else 'consumer'
            need(all(r.get(k)==b.get(role+s) for k,s in [('pid',''),('inc','_inc'),('vm','_vm')]), 'message actor identity')
            need(r.get('message')=='0' and r.get('slot_generation')=='1', 'message identity')
        if a:
            need(raw.index('[FL-TRANSFER] event=publish generation='+g+' ') < raw.index('[FL-TRANSFER] event=ack generation='+g+' '), 'ack before publication')
    bells=rows(raw,'[FL-BELL] ')
    registrations=[r for r in bells if r.get('event')=='registered']
    completions=[r for r in bells if r.get('event')=='complete']
    takes=[r for r in bells if r.get('event')=='take']
    need(len(registrations)==len(completions)==len(takes)==17, 'notification accounting')
    remote_wakes=0
    for reg, comp, take in zip(registrations,completions,takes):
        need(all(reg.get(k)==comp.get(k)==take.get(k) for k in ('generation','consumer','inc','vm')), 'notification identity')
        need(comp.get('result')==take.get('result') and comp.get('sequence')==take.get('sequence') and comp.get('blocked')=='1', 'notification completion')
        b=gens.get(reg.get('generation'),{})
        need(all(reg.get(k)==b.get('consumer'+suffix) for k,suffix in [('consumer',''),('inc','_inc'),('vm','_vm')]), 'unbound notification')
        remote_wakes += reg.get('cpu')!=comp.get('cpu')
    need(remote_wakes>=16, 'remote consumer progress missing')
    parks=rows(raw,'[FL-BELL-PARK] ')
    need(len(parks)==17 and all(all(r.get(k)==v for k,v in {'parked':'1','owner':'255','queued':'255','pins':'0','ready':'0'}.items()) for r in parks), 'physical consumer parking')
    resources=rows(raw,'[FL-BELL-RESOURCE] ')
    need(len(resources)>=20 and all(all(r.get(k)=='0' for k in ('active','waiters','receipts','pending')) for r in resources if r.get('active')=='0'), 'terminal Doorbell inventory')
    need(resources and all(resources[-1].get(k)=='0' for k in ('active','waiters','receipts','pending')), 'final Doorbell inventory')
    census=rows(raw,'[FL-CENSUS] ')
    need(len(census)==20 and all(all(r.get(k)=='0' for k in ('objects','views','frames')) for r in census), 'final mapping/grant ownership')
    cycles=[r for r in census if 500<=n(r,'checkpoint')<=507]
    need(len(cycles)==8 and all(len({r.get(k) for r in cycles})==1 for k in ('shm','tables')), 'repeated-cycle frame drift')
    # Exact per-object proof, reused M2 trace events. No remote ACK is invented
    # for an empty target mask. A genuine remote active-VM case is mandatory.
    maps=rows(raw,'[FL-MAP] ');frames=rows(raw,'[FL-FRAME] ');tlbs=rows(raw,'[FL-TLB] ')
    backs=[r for r in frames if r.get('event')=='backing'];objects=[]
    need(len(backs)==60, '60 physical backing objects required')
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
    need(len([r for r in maps if r.get('event')=='map'])==120, 'unaccounted mapping')
    remote=sum(r.get('event')=='request' and n(r,'targets')>0 for r in tlbs)
    zero=sum(r.get('event')=='request' and n(r,'targets')==0 for r in tlbs)
    need(remote>0 and zero>0, 'remote/zero target distinction')
    need(any(r.get('event')=='ack' and r.get('flushed')=='1' for r in tlbs), 'no actual remote flush')
    faults=rows(raw,'[FL-PF] ');access=rows(raw,'[FL-ACCESS] ')
    need(len(faults)==len(access)==1 and faults[0].get('pid')==access[0].get('pid') and faults[0].get('va')==access[0].get('va') and faults[0].get('error')=='4', 'remote stale-view fault')
    need(raw.count('[FAULT] #14 ')==1 and raw.count('[FAULT] Page Fault at ')==1 and not any(m in raw for m in ('[FAULT] #0 ','[FAULT] #6 ','[FAULT] #13 ')), 'unexpected hardware fault')
    for group in (cases,bindings,pubs,copies,resources,census,maps,frames,tlbs,faults,access,bells,parks):
        need(not any('_duplicate' in r for r in group), 'duplicate field')
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'channels':len(bindings),
            'publications':sum(r.get('event')=='publish' for r in pubs),'acknowledgements':sum(r.get('event')=='ack' for r in pubs),
            'private_copies':len(copies),'copies':copies,'remote_wakes':remote_wakes,'mapping':{'objects':objects,'remote_unmaps':remote,'zero_target_unmaps':zero},'cases':cases}
