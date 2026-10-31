#!/usr/bin/env python3
"""Fail-closed native authenticated cancellation and original-caller exit evidence."""
import re


def assess(raw, cpus, fixture, mode, *, identity=None, failure=None, object_id=1):
    errors = []
    pairs = [(line, re.findall(r'(\w+)=(\S+)',line)) for line in raw.splitlines()]
    records = []
    for line, fields in pairs:
        if not line.startswith(('[AE-','[CE-')): continue
        r = dict(fields)
        if len(r) != len(fields): errors.append('duplicate field')
        if r.get('object',r.get('grant')) != str(object_id): continue
        r['_kind'] = line.split()[0]; records.append(r)
    def require(value, why):
        if not value: errors.append(why)
    def rows(kind,event=None):
        return [r for r in records if r['_kind']=='['+kind+']' and (event is None or r.get('event')==event)]
    def one(kind,event=None):
        found=rows(kind,event); require(len(found)==1,f'exactly one {kind}/{event} required')
        return found[0] if len(found)==1 else {}
    def n(row,key):
        try: return int(row[key])
        except (KeyError,ValueError,TypeError): errors.append('invalid '+key); return -1
    def before(a,b): return n(a,'seq') < n(b,'seq')
    def same(a,b,keys): return all(a.get(k) is not None and a.get(k)==b.get(k) for k in keys)
    require(not any(x in raw for x in ('PANIC','panicked at','checkpoint=FAIL')), 'guest fault')
    g=one('AE-GRANT','selected'); retirement=one('AE-GRANT','retired')
    committed=one('CE-STATE','committed'); deferred=one('CE-STATE','deferred')
    require(all(n(g,k)>0 for k in ('issuer','issuer_inc','issuer_vm','caller','caller_inc','caller_vm','frame','endpoint','generation','session','epoch')), 'invalid authenticated binding')
    require(g.get('object')==g.get('grant') and g.get('views')=='2', 'wrong grant/object/views')
    require(committed.get('pid')==g.get('issuer') and committed.get('inc')==g.get('issuer_inc') and committed.get('vm')==g.get('issuer_vm') and committed.get('auth')=='1', 'wrong privileged Account identity')
    require(all(committed.get(k)=='1' for k in ('receipts','grants','deliveries','maps')), 'real committed outstanding inventory missing')
    target=one('CE-TARGET','committed')
    require(target.get('caller')==g.get('caller') and target.get('inc')==g.get('caller_inc') and same(target,g,('endpoint','generation')) and target.get('live')=='1', 'exact live original target mismatch')
    require(deferred.get('deferred')==('0' if mode==1 else '1') and before(committed,deferred), 'original deferred target not retained')
    require(all(retirement.get(k)=='0' for k in ('grants','receipts','pending_deliveries')), 'retirement metadata incomplete')
    late=one('CE-STATE','late-denied'); unmapped=one('CE-STATE','account-unmapped')
    require(before(deferred,late) and before(late,unmapped), 'late response/Account unmap order')
    require(all(late.get(k)=='0' for k in ('grants','deliveries','receipts')), 'original grant/receipt/delivery not cleaned before late response')
    require(unmapped.get('maps')=='0', 'Account view leaked')
    reaping=one('CE-REAP','reap-ready'); reaped=one('CE-REAP','reap-clean')
    for r in (reaping,reaped):
        require(r.get('pid')==g.get('caller') and r.get('inc')==g.get('caller_inc') and r.get('vm')==g.get('caller_vm'), 'original caller reaping identity mismatch')
        require(r.get('active')=='0' and r.get('owning')==r.get('queued')=='255', 'reap before execution/VM quiescence')
    require(all(reaped.get(k)=='0' for k in ('maps','pending','outcome','reply','deferred','endpoints')), 'caller cleanup incomplete')
    require(before(reaping,reaped), 'reap order')
    if mode in (4,5,11,12,13):
        require(reaping.get('maps')=='1', 'caller exited after premature checked release')
    else:
        require(reaping.get('maps')=='0', 'explicit caller release absent')
    terminal=rows('AE-TERMINAL')
    completion_won = mode == 11 or mode == 13 and any(
        line.startswith('[EX-RACE]') and dict(fields).get('object') == str(object_id)
        and dict(fields).get('event') == 'returned' and dict(fields).get('actor') == '1'
        and dict(fields).get('result') == '0' for line, fields in pairs)
    if mode==4 or mode >= 11 and not completion_won:
        require(not terminal,'dead caller received terminal publication')
        require(before(reaped,late) and before(retirement,reaped), 'exit-wins completion before caller reaping')
    else:
        require(len(terminal)==1, 'exactly one terminal required')
        if len(terminal)==1:
            completion_won = completion_won or mode == 8 or mode == 10 and any(
                line.startswith('[RC-RACE]') and dict(fields).get('object') == str(object_id)
                and dict(fields).get('event') == 'returned' and dict(fields).get('actor') == '1'
                and dict(fields).get('result') == '0' for line, fields in pairs)
            t=terminal[0]; expected='ReplyDelivered' if mode in (3,5) or completion_won else 'ExplicitlyCancelled' if mode >= 8 else 'PeerClosed' if mode==7 else 'DeadlineExpired'
            require(t.get('caller')==g.get('caller') and t.get('inc')==g.get('caller_inc') and same(t,g,('endpoint','generation')) and t.get('outcome')==expected+'('+g.get('generation','')+')', 'terminal outcome/identity/generation mismatch')
            require(before(t,reaped), 'terminal published after reap')
    if mode in (3,5):
        published=one('CE-STATE','published')
        require(before(published,late) and published.get('deliveries')=='0', 'reply commitment missing')
        e=one('CE-STATE','exit-with-reply' if mode==5 else 'cancel-after-commit-before-collect')
        require(e.get('pid')==g.get('caller') and e.get('reply')=='1' and e.get('outcome')=='Some(ReplyDelivered('+g.get('generation','')+'))', 'committed reply before collection not established')
        if mode==3:
            preserved=one('CE-STATE','cancel-preserved-reply')
            require(preserved.get('reply')==e.get('reply') and preserved.get('outcome')==e.get('outcome') and before(e,preserved), 'explicit cancellation replaced committed terminal')
    if mode not in (6,7):
        fresh=one('CE-TARGET','fresh')
        require(fresh.get('caller')!=g.get('caller') and fresh.get('inc')!=g.get('caller_inc') and fresh.get('endpoint')==g.get('endpoint') and fresh.get('generation')!=g.get('generation') and fresh.get('live')=='1', 'foreign fresh call exact binding missing')
    else:
        closed=one('CE-STATE','endpoint-closed')
        require(before(closed,late),'endpoint closure missing')
    if mode in (1,2,6):
        cancelled=one('CE-STATE','cancel-observed')
        require(all(cancelled.get(k)=='0' for k in ('grants','receipts','deliveries')), 'cancellation receipt or delivery survived settlement')
    if mode!=4:
        consumed=rows('CE-STATE','caller-consumed')
        require(len(consumed)==(0 if mode==5 or mode>=11 else 1), 'caller terminal collection count mismatch')
        if consumed:
            require(all(consumed[0].get(k)=='0' for k in ('pending','reply','deferred')) and consumed[0].get('outcome')=='None', 'collected caller terminal retained')
    workers = [dict(fields) for line,fields in pairs if line.startswith('[AE-SNAPSHOT]') and dict(fields).get('event')=='worker-ready' and dict(fields).get('vm')==g.get('caller_vm')]
    require(len(workers)==1 and workers[0].get('pid')!=g.get('caller'), 'actual caller VM borrower missing')
    if len(workers)==1:
        w=workers[0]
        worker_reap=re.search(r'\[SCHED\] process_reaped pid='+re.escape(w['pid'])+r'\b',raw)
        require(worker_reap is not None, 'actual borrower not reaped')
        if mode in (4,5,11,12,13):
            require('[SCHED] shared-address-space borrower pid='+w['pid']+' terminated reason=address-space-owner-exit' in raw, 'original exit did not terminate the actual VM borrower')
            parent_ready=next((line for line,_ in pairs if line.startswith('[CE-REAP]') and 'event=reap-ready ' in line and 'pid='+g.get('caller','')+' ' in line),'')
            require(worker_reap is not None and bool(parent_ready) and worker_reap.start()<raw.find(parent_ready), 'parent reclaimed before actual borrower reap')
    users=[dict(fields) for line,fields in pairs if line.startswith('[CE-USER]') and dict(fields).get('child')==g.get('caller')]
    def user(event):
        found=[r for r in users if r.get('event')==event and r.get('mode')==str(mode)]
        require(len(found)==1, 'missing/duplicate userspace '+event)
        return found[0] if len(found)==1 else {}
    supervisor=user('foreign-live-authority-denied')
    require(supervisor.get('pid')!=g.get('caller') and supervisor.get('pid')!=g.get('issuer'), 'foreign process test not independent')
    require(user('caller-endpoint-retired').get('pid')==supervisor.get('pid'), 'caller endpoint retirement not tested by supervisor')
    if mode not in (6,7):
        require(user('foreign-adoption-denied').get('pid')==g.get('issuer'), 'late original grant adoption not denied by live Account')
        require(fresh.get('caller')==supervisor.get('pid'), 'fresh grant request is not exact foreign supervisor')
    final=one('CE-STATE','supervisor-clean')
    require(final.get('pid')==supervisor.get('pid') and all(final.get(k)=='0' for k in ('pending','reply','deferred','maps','receipts','grants','deliveries','sessions')) and final.get('outcome')=='None', 'complete-cycle inventory not cleaned')
    mappings = rows('AE-MAP', 'existing')
    require(len(mappings) == 2 and {m.get('pid') for m in mappings} == {g.get('issuer'),g.get('caller')}, 'exact original SHM ownership missing')
    releases = []
    remote_acks = 0
    for m in mappings:
        prefix = 'issuer' if m.get('pid') == g.get('issuer') else 'caller'
        require(m.get('inc') == g.get(prefix+'_inc') and m.get('vm') == g.get(prefix+'_vm'), 'mapping ownership incarnation/VM incorrect')
        require(m.get('frame') == g.get('frame') and m.get('object') == g.get('object') and m.get('bytes') == '4096' and n(m,'va') > 0, 'mapping/frame mismatch')
        unmaps = [r for r in rows('AE-UNMAP') if same(r,m,('pid','inc','vm','va'))]
        by_event = {event: [r for r in unmaps if r.get('event') == event] for event in ('request','complete','retained','transfer-to-pmm')}
        require(all(same(r,g,('object','frame')) for r in unmaps), 'unmap changed object/frame identity')
        require(len(by_event['request']) == len(by_event['complete']) == 1 and len(unmaps) == 3, 'mapping unmap transitions missing or duplicated')
        if len(by_event['request']) != 1 or len(by_event['complete']) != 1:
            continue
        request, complete = by_event['request'][0], by_event['complete'][0]
        expected_views = '2' if by_event['retained'] else '1'
        require(request.get('views') == complete.get('views') == expected_views, 'unmap began with incorrect reference count')
        if m.get('pid') == g.get('caller') and mode in (4,5,11,12,13):
            require(before(reaping,request) and before(complete,reaped),'exit unmap outside quiescent reaping')
        release = by_event['retained'] + by_event['transfer-to-pmm']
        require(len(release) == 1, 'mapping ownership release missing')
        if len(release) != 1:
            continue
        release = release[0]
        releases.append(release)
        require(before(m,request) and before(request,complete) and before(complete,release), 'mapping release precedes verified unmap')
        tlb = [r for r in rows('AE-TLB') if same(r,m,('vm','va'))]
        cleared = [r for r in tlb if r.get('event') == 'pte-cleared']
        requests = [r for r in tlb if r.get('event') == 'request']
        completions = [r for r in tlb if r.get('event') == 'complete']
        require(len(cleared) == len(requests) == len(completions) == 1, 'PTE/TLB invalidation incomplete or duplicated')
        if len(cleared) != 1 or len(requests) != 1 or len(completions) != 1:
            continue
        c, q, d = cleared[0], requests[0], completions[0]
        require(before(request,c) and before(c,q) and before(q,d) and before(d,complete), 'PTE/TLB completion ordering incorrect')
        require(n(q,'request') > 0 and same(q,d,('request','targets','frame','object')) and q.get('frame') == g.get('frame'), 'TLB correlation incorrect')
        targets = n(q,'targets')
        origin = n(q,'cpu')
        require(0 <= origin < cpus and 0 <= targets < (1 << cpus) and not targets & (1 << origin), 'invalid remote target mask')
        acks = [r for r in tlb if r.get('event') == 'ack-observed']
        expected = {str(cpu) for cpu in range(cpus) if targets & (1 << cpu)} if targets >= 0 else set()
        require(len(acks) == len(expected) and {a.get('ack_cpu') for a in acks} == expected, 'missing/duplicate remote ACK')
        for ack in acks:
            require(same(q,ack,('request','targets','frame','object')) and ack.get('produced') == q.get('request') and ack.get('flushed') in ('0','1') and before(q,ack) and before(ack,d), 'remote ACK does not precede completion or lacks independent handler witness')
        remote_acks += len(acks)
    require(len(rows('AE-UNMAP')) == 6 and len(releases) == 2, 'unexpected mapping activity')
    retained, transfer = one('AE-UNMAP', 'retained'), one('AE-UNMAP', 'transfer-to-pmm')
    require(retained.get('views') == '1' and transfer.get('views') == '0' and before(retained,transfer), 'backing transfer while views remain')
    physical = one('AE-PMM','free')
    require(physical.get('class') == '0' and same(physical,g,('frame','object')) and before(transfer,physical), 'physical free before final ownership transfer')
    for completion in rows('AE-TLB','complete'):
        require(before(completion,physical), 'physical free before TLB completion')
    claims = rows('AE-PMM','claim')
    require(len(claims) <= 1, 'duplicate first-reuse observation')
    if claims:
        require(same(claims[0],g,('frame','object')) and before(physical,claims[0]), 'reuse before free')
    for pid in (g.get('issuer'),g.get('caller')):
        require(pid is not None and re.search(r'\[SCHED\] process_reaped pid='+str(pid)+r'\b',raw), 'participant not actually reaped')
    require(identity is not None and all(identity.get(k) for k in ('source_matches','services_match','iso_matches','elf_matches')), 'source/image identity absent')
    require(failure is None,'driver failure: '+str(failure))
    seqs=[n(r,'seq') for r in records if 'seq' in r]
    require(seqs==sorted(set(seqs)) and all(x>0 for x in seqs),'duplicate/reordered trace')
    require(all(0<=n(r,'cpu')<cpus for r in records if 'cpu' in r),'invalid CPU')
    valid=not errors
    return dict(status='PASS' if valid else 'FAIL', errors=errors, grant=g, mode=mode, terminal=terminal, retirement=retirement, physical=physical, remote_acknowledgements=remote_acks, remote_flushes=sum(r.get('flushed')=='1' for r in rows('AE-TLB','ack-observed')), zero_target_unmaps=sum(r.get('targets')=='0' for r in rows('AE-TLB','request')), records=records)


def assess_cycles(raw,cpus,fixtures,*,mode,identity=None,failure=None):
    errors=[]
    selected=[dict(re.findall(r'(\w+)=(\S+)',l)) for l in raw.splitlines() if l.startswith('[AE-GRANT]') and 'event=selected ' in l]
    if len(selected)!=len(fixtures) or len(set(fixtures))!=len(fixtures): errors.append('fixture/object cycle count mismatch')
    cycles=[assess(raw,cpus,f,mode,identity=identity,failure=failure,object_id=i+1) for i,f in enumerate(fixtures)]
    for i,c in enumerate(cycles): errors.extend(f'cycle {i+1}: '+e for e in c['errors'])
    clients=[dict(re.findall(r'(\w+)=(\S+)',l)) for l in raw.splitlines() if l.startswith('[SESSION-E3-CLIENT]')]
    for f in fixtures:
        for e in ('request-start','fixture-complete'):
            if len([r for r in clients if r.get('fixture')==f and r.get('checkpoint')==e])!=1: errors.append('missing/duplicate actual fixture completion')
    if len({(g.get('issuer'),g.get('issuer_inc')) for g in selected})!=len(selected): errors.append('Account incarnation reused')
    if len({(g.get('caller'),g.get('caller_inc'),g.get('caller_vm')) for g in selected})!=len(selected): errors.append('caller lifetime reused')
    valid=not errors
    return dict(status='PASS' if valid else 'FAIL',classification='FORCED_AND_PASSED' if valid else 'FORCED_AND_FAILED',errors=errors,terminal_protocol='GUEST_VALIDATED_PENDING_HOST_ACK' if valid else 'BLOCKED',host_terminal_acknowledged=False,fixture_ids=fixtures,cycles=cycles,mode=mode,identity=identity,remote_acknowledgements=sum(c['remote_acknowledgements'] for c in cycles),remote_flushes=sum(c['remote_flushes'] for c in cycles),concurrent_invocation_overlap='NOT_RUN',milestone='PARTIAL',production_fastlane='BLOCKED')
