#!/usr/bin/env python3
"""Independent R2 receipt assessment. A legacy M1 pass cannot satisfy this gate."""
import json
import re
from pathlib import Path

EVENTS = dict(enumerate(['EXIT_STARTED','EXIT_GUARD_ACQUIRED','SWEEP_OBSERVED','IDENTITY_VALIDATED',
    'RETIREMENT_COMMITTED','ENDPOINT_CLOSED','TERMINAL_CANDIDATE','TERMINAL_COMMITTED',
    'CLEANUP_STARTED','CLEANUP_COMPLETED','ENDPOINT_CLOSE_ENTERED','ENDPOINT_BROKER_REMOVED',
    'EXIT_NESTED_REJECTED','EXIT_GUARD_RELEASED','RETIRE_PROCESS_ENTERED','TERMINAL_REJECTED','RETIREMENT_CONTENDER_ACQUIRED','RETIREMENT_CONTENDER_REJECTED','RETIREMENT_CONTENDER_PROBED'],1))
FIELDS = set('seq generation operation pid inc vm session epoch channel event cpu outcome state detail if'.split())

def fields(line, prefix):
    tail=line.split(prefix,1)[1].strip()
    pairs=[word.split('=',1) for word in tail.split()]
    if any(len(pair)!=2 for pair in pairs): raise ValueError('malformed fields')
    if len({p[0] for p in pairs})!=len(pairs): raise ValueError('duplicate field')
    return dict(pairs)

def assess(raw, generation, build, operation=1):
    errors=[]; trace=[]; proof={}
    def require(ok,message):
        if not ok: errors.append(message)
    try:
        headers=[fields(l,'[KFP-LC-HEADER]') for l in raw.splitlines() if '[KFP-LC-HEADER]' in l]
        ends=[fields(l,'[KFP-LC-END]') for l in raw.splitlines() if '[KFP-LC-END]' in l]
        require(len(headers)==len(ends)==1,'missing/duplicate trace envelope')
        if len(headers)!=1 or len(ends)!=1: raise ValueError('no complete envelope')
        require(headers[0]==dict(generation=str(generation),operation=str(operation),overflow='0',build=build),'stale build/generation or overflow')
        for line in raw.splitlines():
            if '[KFP-LC]' not in line: continue
            row=fields(line,'[KFP-LC]')
            require(set(row)==FIELDS,'missing/unknown receipt field')
            trace.append({k:int(v) for k,v in row.items()})
        require(bool(trace),'empty native trace')
        if not trace: raise ValueError('no native receipts')
        require(ends[0]==dict(generation=str(generation),count=str(len(trace)),overflow='0'),'incomplete trace or overflow')
        require([r['seq'] for r in trace]==list(range(1,len(trace)+1)),'noncontiguous/duplicate publication')
        identity_keys='generation operation pid inc vm session epoch channel'.split()
        identity={k:trace[0][k] for k in identity_keys}
        require(identity['generation']==generation and identity['operation']==operation,'stale operation identity')
        require(all(v>0 for v in identity.values()),'zero identity')
        require(all({k:r[k] for k in identity_keys}==identity for r in trace),'changed identity or stale generation')
        require(all(r['event'] in EVENTS and r['cpu'] in (0,1) for r in trace),'invalid event/CPU')
        bindings=[fields(l,'[FL-BIND]') for l in raw.splitlines() if '[FL-BIND]' in l]
        require(any(b.get('generation')==str(identity['channel']) and b.get('session')==str(identity['session']) and b.get('epoch')==str(identity['epoch']) and b.get('consumer')==str(identity['pid']) and b.get('consumer_inc')==str(identity['inc']) and b.get('consumer_vm')==str(identity['vm']) and b.get('op')=='3' for b in bindings),'no independent authenticated consumer binding')
        def events(n): return [r for r in trace if r['event']==n]
        def one(n):
            rows=events(n);require(len(rows)==1,'missing/duplicate '+EVENTS[n]);return rows[0] if rows else {'seq':0}
        guard=one(2);start=one(1);terminal=one(8);retirement=one(5);cleanup=one(9);done=one(10)
        exact=[r for r in events(3) if r['outcome']==1 and r['detail']==1]
        require(len(exact)==1,'no unique sweep observing exact exiting participant while live')
        sweep=exact[0] if exact else {'seq':0}
        validated=[r for r in events(4) if r['seq']==sweep['seq']+1 and r['outcome']==2 and r['detail']==1]
        require(len(validated)==1,'real classifier did not validate exact participant as ProcessRetired')
        validation=validated[0] if validated else {'seq':0}
        require(guard['seq']<start['seq']<sweep['seq']<validation['seq']<terminal['seq']<retirement['seq']<cleanup['seq']<done['seq'],'invalid native lifecycle order')
        require(start.get('state')==3 and start.get('detail')==1,'exit was not committed Finished/pending')
        require(terminal.get('outcome')==retirement.get('outcome')==2 and terminal.get('state')!=4 and retirement.get('state')==4,'wrong terminal or retirement state')
        require(all(r['outcome']==2 for r in events(8)),'IdentityChanged or unauthorized terminal publication')
        require(all(r['seq']>=terminal['seq'] or r['event']!=5 for r in trace),'retirement already committed before observation')
        require(cleanup.get('state')==3 and cleanup.get('detail')==1 and done.get('state')==4 and done.get('detail')==0,'cleanup ownership/pending/stack completion failed')
        enter=one(11);hooks=events(15)
        require(len(hooks)==2,'expected native exit and reaper retirement hook entries')
        hook=hooks[0] if hooks else {'seq':0}
        require(retirement['seq']<enter['seq']<hook['seq'],'early sweep did not precede native retirement hook')
        require(len(hooks)==2 and cleanup['seq']<hooks[1]['seq']<done['seq'],'reaper retirement hook absent/outside cleanup')
        # Multiple actual hook invocations are required by the native contract.
        # They must each lose to the single earlier terminal publication.
        for h in hooks:
            require(any(r['seq']==h['seq']+1 and r['event']==7 and r['outcome']==2 for r in trace),'retirement hook lacks actual candidate')
            require(any(r['seq']==h['seq']+2 and r['event']==16 and r['outcome']==2 and r['detail']==2 for r in trace),'repeat retirement hook did not preserve committed ProcessRetired')
        for c in events(7):
            require(any(r['seq']==c['seq']+1 and r['event'] in (8,16) and r['outcome']==c['outcome'] for r in trace),'unresolved terminal candidate')
        removed=events(12);closed=events(6)
        require(bool(removed) and len({r['detail'] for r in removed})==len(removed),'no exact native endpoint broker removal')
        require({r['detail'] for r in closed}=={r['detail'] for r in removed} and len(closed)==len(removed),'endpoint closure identity mismatch/duplicate')
        require(all(next(x['seq'] for x in removed if x['detail']==r['detail'])<r['seq']<cleanup['seq'] for r in closed),'native endpoint cleanup order')
        require(any(r['outcome']==1 and r['detail']==2 and r['seq']>retirement['seq'] for r in events(16)),'competing Revoked candidate not rejected with ProcessRetired preserved')
        require(len(events(14))==1,'exit guard post-drop observation absent/duplicate')
        require(not events(13),'unexpected nested exit rejection in R2')
        require('[KFP-R2] event=complete build='+build+' generation='+str(generation) in raw,'dedicated lifecycle payload incomplete')
        require('[KFP-SEC] event=complete suite='+('r4' if operation==2 else 'r2') in raw,'authenticated controller incomplete')
        require(not re.search(r'PANIC|panicked at|checkpoint=FAIL',raw),'guest assertion')
        if operation==2:
            probe=one(19);acquired=one(17);rejected=one(18)
            require(probe.get('outcome')==1 and probe['seq']<guard['seq'],'no independently failed native SIGKILL contender probe before exit guard')
            require(all(r['seq']<acquired['seq'] for r in closed) and acquired['seq']<rejected['seq'],'SIGKILL did not acquire/reject after native endpoint closure')
            require(acquired.get('state') in (3,4) and rejected.get('state') in (3,4) and rejected.get('outcome')==1,'native already-finished guard was not preserved')
            require(probe.get('cpu')!=guard.get('cpu') and acquired.get('cpu')==probe.get('cpu'),'native retirement contender CPU identity changed')
            races=[fields(l,'[FL-RACE]') for l in raw.splitlines() if '[FL-RACE]' in l]
            races=[r for r in races if r.get('mode')=='6' and r.get('generation')==str(identity['channel'])]
            require(any(r.get('event')=='lock-contended' and r.get('actor')=='1' and r.get('seq')==str(probe.get('detail')) for r in races),'missing independently logged real failed scheduler try_lock')
            require(any(r.get('event')=='exit-syscall' and r.get('actor')=='2' and r.get('pid')==str(identity['pid']) for r in races),'native self-exit entry absent')
            require('[KFP-R4] event=native-close-versus-sigkill-complete' in raw,'R4 dedicated fixture incomplete')
        proof={'identity':identity,'causal_sequence':[{**r,'event_name':EVENTS[r['event']]} for r in (guard,start,sweep,validation,terminal,retirement,enter,hook,cleanup,done) if 'event' in r],
               'terminal_commits':len(events(8)), 'cleanup_commits':len(events(10)),
               'nested_exit':'NOT_OBSERVED', 'endpoint_ids':[r['detail'] for r in removed]}
    except (ValueError,KeyError,IndexError,TypeError) as error:
        errors.append(str(error))
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'proof':proof,'events':trace,
            'scope':'R4 native owner-death endpoint closure vs losing SIGKILL' if operation==2 else 'R2 self-exit / real early sweep; no R6 overlap or nested-entry proof'}

if __name__=='__main__':
    import sys
    manifest=Path(sys.argv[1]);data=json.loads(manifest.read_text())
    result=assess(Path(data['native'][0]['serial']).read_text(),data['test_generation'],data['build_id'],operation=2 if data['suite']=='r4' else 1)
    print(json.dumps(result,indent=2));raise SystemExit(result['status']!='PASS')
