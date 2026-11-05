"""M6.1A fail-closed, independent actor/window, clock and retirement checks."""
from math import ceil
from fastlane_channel_validate import rows
from fastlane_m60r_receipt import retirement

def clock(a,b):
    try:
        if a['cpu']!=b['cpu']: return 'migration'
        if int(a['epoch'])<=0 or a['epoch']!=b['epoch']: return 'epoch'
        if any(a[k]!=b[k] for k in ('hz','ppm','flags','spread')): return 'calibration'
        if not int(a['hz']) or int(a['flags'])&36!=36 or not 0<int(a['ppm'])<=20000: return 'uncertainty'
        n=int(b['ns'])-int(a['ns'])
        return n if n>0 else 'backward-or-zero'
    except (KeyError,ValueError,TypeError): return 'malformed'

def assess(raw,cpus):
    errors=[]; accepted=[]; rejected=[]
    def need(ok,why):
        if not ok: errors.append(why)
    try:
        need(len(raw)<=32*1024*1024,'bounded receipt')
        need(not any(m in raw for m in ('PANIC','panicked at','[FAULT]','checkpoint=FAIL')),'guest assertion')
        prefixes=['[FL-M61A'+s+'] ' for s in ('','-P','-R','-STAMP','-CPU','-CONTROL','-NEG','-MATCH','-WARM','-FOREIGN','-ABORT')]
        groups=[rows(raw,p) for p in prefixes]
        need(not any('_duplicate' in r for g in groups for r in g),'duplicate fields')
        terminal,ps,rs,stamps,counters,controls,neg,matches,warm,foreign,aborts=groups
        need(len(terminal)==1 and terminal[0].get('case')=='complete' and terminal[0].get('resources')=='0' and terminal[0].get('production')=='DISABLED','terminal')
        need(len(ps)==len(rs)==1,'workload population')
        p=ps[0];r=rs[0];pids={p['producer'],p['consumer']};g=p['generation']
        need([int(p[k]) for k in ('warm','measured','sent','replies','bytes')]==[16,128,144,144,32] and p['integrity']=='exact','request acknowledgements')
        need([int(r[k]) for k in ('received','verified','acks')]==[144]*3 and r['pid']==p['consumer'] and r['peer']==p['producer'],'consumer integrity')
        need(terminal[0]['pid']==p['producer'],'terminal actor')
        binds=[b for b in rows(raw,'[FL-BIND] ') if b.get('op')=='2']
        need(len(binds)==1,'binding inventory');binding=binds[0]
        need(binding['generation']==g and binding['producer']==p['producer'] and binding['consumer']==p['consumer'],'bound actors')
        identities={binding[role]:(binding[role+'_inc'],binding[role+'_vm']) for role in ('producer','consumer')}
        def auth(row): return row.get('pid') in identities and (row.get('inc'),row.get('vm'))==identities[row['pid']] and row.get('generation',g)==g
        need(all(auth(t) for t in stamps+counters+controls),'kernel authenticated identities')
        need(len(stamps)==7 and len({(t['pid'],t['w'],t['event']) for t in stamps})==7,'stamp inventory')
        need(len(counters)==3 and {(c['pid'],c['w']) for c in counters}=={(p['producer'],'1'),(p['consumer'],'1'),(p['consumer'],'2')},'window population')
        for c in counters:
            pair=[t for t in stamps if t['pid']==c['pid'] and t['w']==c['w']]
            a=[t for t in pair if t['event']=='start'];b=[t for t in pair if t['event']=='stop']
            need(len(pair)==2 and len(a)==len(b)==1,'matched endpoints')
            if len(a)!=1 or len(b)!=1:continue
            a,b=a[0],b[0];q=clock(a,b)
            need(int(b['runtime'])>=int(a['runtime']) and int(c['charged'])==int(b['runtime'])-int(a['runtime']),'scheduler delta')
            need(c['cpu_start']==a['cpu'] and c['cpu_stop']==b['cpu'],'CPU identity')
            need(c['overflow']=='0' and c['discontinuities']=='0','counter discontinuity')
            if c['w']=='1':
                need(isinstance(q,int) and c['wall_valid']=='1' and c['valid']=='1' and c['migrated']=='0' and a['cpu']==b['cpu']=='0','qualified same CPU pair')
                need(isinstance(q,int) and int(c['wall'])==q and 0<int(c['charged'])<=q and int(c['switches'])>0 and c['reconcile']=='consistent','CPU reconciliation')
                accepted.append({**c,'scale_uncertainty_ns':ceil(q*int(a['ppm'])/1000000)+1 if isinstance(q,int) else None,'start':a,'stop':b})
            else:
                need(q=='migration' and a['cpu']=='0' and b['cpu']=='1' and c['migrated']=='1' and c['wall_valid']==c['valid']=='0' and c['reconcile']=='not-qualified','migration refusal')
                rejected.append({'reason':'migration','accounting_status':'NOT_QUALIFIED',**c})
        union=None
        if len(accepted)==2:
            starts=[c['start'] for c in accepted];stops=[c['stop'] for c in accepted]
            a=min(starts,key=lambda t:int(t['ns']));b=max(stops,key=lambda t:int(t['ns']))
            duration=clock(a,b);charge=sum(int(c['charged']) for c in accepted)
            need(isinstance(duration,int) and charge<=duration,'combined actor reconciliation')
            union={'wall_ns':duration,'charged_sum_ns':charge,'status':'consistent' if isinstance(duration,int) and charge<=duration else 'inconsistent'}
        need(len(controls)==len(neg)==len(matches)==2 and all({r['pid'] for r in group}==pids for group in (controls,neg,matches)),'control actors')
        need(all(all(c.get(k)=='reject' for k in ('stale_inc','foreign','stale_vm','exhausted','invalid_pair','cross_cpu')) for c in controls),'synthetic native controls')
        need(all(c.get('missing_start')=='7' and c.get('stale_generation')=='2' for c in neg),'negative selector results')
        need(all(c.get('w')=='1' and all(c.get(k)==v for k,v in [('duplicate_start','3'),('wrong_window','2'),('duplicate_stop','3'),('stale_window','2')]) for c in matches),'window rejection codes')
        need(len(foreign)==1 and foreign[0]['pid'] not in pids and foreign[0].get('start')==foreign[0].get('stop')=='1','foreign native actor')
        need(len(warm)==1 and [warm[0].get(k) for k in ('completed','measured','bytes','outstanding')]==['16','128','32','1'],'warm boundary')
        need(len(aborts)==1 and aborts[0].get('w')=='3' and aborts[0].get('generation')==g and aborts[0].get('qualified')=='0','actor exit incomplete')
        retired=retirement(raw,cpus,expected_backings=3,checkpoints=(999,));errors+=retired['errors']
        # Kernel-authenticated cleanup and exact reap for all three actors.
        clean=rows(raw,'[SESSION-E3] ');reaped=rows(raw,'[SCHED] process_reaped ')
        for pid in pids|{foreign[0]['pid']}:
            c=[x for x in clean if x.get('checkpoint')=='fixture-ipc-cleaned' and x.get('pid')==pid]
            need(len(c)==1,'exact IPC cleanup actor')
            if c:
                need(all(c[0].get(k)==v for k,v in {'owned_endpoints':'0','pending_call':'none','reply_target':'none','deferred_targets':'0','pending_commits':'0','snapshot':'serialized'}.items()),'no outstanding IPC state')
                if pid in identities: need((c[0]['incarnation'],c[0]['vm'])==identities[pid],'cleanup identity')
            rr=[x for x in reaped if x.get('pid')==pid]
            need(len(rr)==1 and rr[0].get('ipc_cleared')=='0','exact actor reap')
        need(auth(aborts[0]) and aborts[0]['pid']==p['consumer'],'aborted actor identity')
        # The incomplete start is distinct from every completed window.
        pending=[t for t in stamps if t['w']=='3']
        need(len(pending)==1 and pending[0]['pid']==p['consumer'] and pending[0]['event']=='start','incomplete endpoint inventory')
        return {'status':'FAIL' if errors else 'PASS','errors':sorted(set(errors)),'qualified':accepted,'reconciliation':union,'rejected_windows':rejected,'retirement':retired,'production_fastlane':'DISABLED','migration_safe_execution':'NOT_QUALIFIED'}
    except (KeyError,ValueError,TypeError,IndexError) as e:
        return {'status':'FAIL','errors':errors+['malformed receipt: '+str(e)],'qualified':accepted,'rejected_windows':rejected}
