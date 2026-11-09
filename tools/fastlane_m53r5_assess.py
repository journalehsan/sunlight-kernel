"""Exact resource joins for the R5 scheduled subset. Never certify absent races."""
import re

MODES = ('r5-owner-exit', 'r5-owner-kill', 'r5-owner-crash', 'r5-owner-exec',
         'r5-peer-exit', 'r5-peer-crash')


def records(raw, prefix):
    result = []
    for index, line in enumerate(raw.splitlines()):
        if not line.startswith(prefix):
            continue
        pairs = re.findall(r'(\w+)=(\S+)', line)
        row = dict(pairs)
        if len(pairs) != len(row):
            raise ValueError('duplicate receipt field')
        row['_line'] = index
        result.append(row)
    return result


def assess(raw, cpus=None, source=None, run=None):
    errors = []
    resources = []
    scenarios = []
    reclamation = {}
    try:
        lines = raw.splitlines()
        terminal = [i for i,line in enumerate(lines) if line.startswith('[R5-USER] case=lifecycle-subset event=complete ')]
        if len(terminal)!=1:
            raise ValueError('missing unique R5 terminal receipt')
        # The original R4.1 regression runs after this closed R5 interval.
        raw = '\n'.join(lines[:terminal[0]+1])
        start = records(raw, '[R5-START] ')
        if len(start) != 1 or not re.fullmatch(r'[a-f0-9]{64}',start[0]['source']) or not re.fullmatch(r'[a-f0-9]{32}',start[0]['run']):
            raise ValueError('missing unique compiled source and invocation identity')
        if any(expected is not None and start[0][key] != str(expected) for key,expected in [('cpus',cpus),('source',source),('run',run)]):
            raise ValueError('stale source/image invocation or CPU topology')
        user = [r for r in records(raw, '[R5-USER] ') if r['event'] != 'before-ud2']
        expected = [(m, e) for m in MODES for e in ('begin', 'peer-attached', 'complete')]
        expected.append(('lifecycle-subset', 'complete'))
        if [(r['case'], r['event']) for r in user] != expected:
            raise ValueError('missing, duplicate or unordered userspace handshakes')
        snapshots = records(raw, '[R5-SNAPSHOT] ')
        if not snapshots or snapshots[0]['label'] != 'baseline' or snapshots[-1]['label'] != 'quiescent':
            raise ValueError('missing baseline/quiescent observations')
        if any(snapshots[i][k] != '0' for i in (0, -1) for k in ('process_bytes', 'session_bytes', 'live_receipts', 'mappings')):
            raise ValueError('test-owned quota/receipt/view drift')
        for r in snapshots:
            expected_bytes = 0 if r['label'] in ('baseline','quiescent') else 4096 if r['label'] in ('control-live','final-view-reclaimed','duplicate-release-denied','iteration-closed') else 8192
            if r['label']=='actor-entry':
                mode = next(m['case'] for m in reversed(user) if m['_line'] < r['_line'] and m['event']=='begin')
                expected_bytes = 8192 if mode.startswith('r5-peer') else 4096
            if int(r['session_bytes']) != expected_bytes:
                raise ValueError('premature quota release or unexpected session charge')
        if len({(r['session'], r['epoch']) for r in snapshots}) != 1:
            raise ValueError('session changed across actors/exec')
        quota = records(raw, '[R5-QUOTA] ')
        views = records(raw, '[R5-VIEW] ')
        physical = records(raw, '[R5-RESOURCE] ')
        tlbs = records(raw, '[R5-TLB] ')
        process = records(raw, '[R5-PROCESS] ')
        identities = {r['resource'] for r in quota}
        if any(r['resource'] not in identities for r in views+physical):
            raise ValueError('unaccounted resource')
        for resource in sorted({int(r['resource']) for r in quota}):
            q = [r for r in quota if int(r['resource']) == resource]
            v = [r for r in views if int(r['resource']) == resource]
            p = [r for r in physical if int(r['resource']) == resource and r['event'] in ('backing','before-reclaim','frame-freed','after-reclaim')]
            charge = [r for r in records(raw,'[R5-CHARGE] ') if int(r['resource'])==resource]
            if len(charge)!=1 or charge[0]['pid']!=q[0]['pid'] or charge[0]['incarnation']!=q[0]['incarnation'] or int(charge[0]['process_before']) < int(q[0]['bytes']):
                raise ValueError('original payer not charged before physical reclamation')
            if [r['event'] for r in q] != ['reserved', 'consumed']:
                raise ValueError('receipt not consumed exactly once')
            if any(q[0][k] != q[1][k] for k in ('pid', 'incarnation', 'bytes')):
                raise ValueError('receipt owner changed')
            if int(q[0]['bytes']) != 4096 or q[0]['session'] != snapshots[0]['session'] or q[0]['epoch'] != snapshots[0]['epoch']:
                raise ValueError('unexpected backing/session')
            if [r['event'] for r in p] != ['backing', 'before-reclaim', 'frame-freed', 'after-reclaim']:
                raise ValueError('missing physical free evidence')
            if p[0]['value'] != p[2]['value'] or int(p[1]['value']) != 1:
                raise ValueError('physical frame identity mismatch')
            if not q[0]['_line'] < p[0]['_line'] < p[1]['_line'] < p[2]['_line'] < p[3]['_line'] < q[1]['_line']:
                raise ValueError('PMM free/receipt order')
            live = {}
            for r in v:
                identity = tuple(r[k] for k in ('pid', 'incarnation', 'vm', 'root', 'va'))
                if r['event'] in ('published', 'attached'):
                    if identity in live:
                        raise ValueError('duplicate view identity')
                    live[identity] = r
                elif r['event'] == 'before-unmap':
                    if identity not in live:
                        raise ValueError('unmap without authorized publication')
                elif r['event'] == 'after-unmap':
                    if identity not in live:
                        raise ValueError('duplicate final unmap')
                    before = [b for b in v if b['event'] == 'before-unmap' and all(b[k] == r[k] for k in ('pid', 'incarnation', 'vm', 'root', 'va'))]
                    if len(before) != 1:
                        raise ValueError('missing unique unmap boundary')
                    ts = [t for t in tlbs if t['vm'] == r['vm'] and t['root'] == r['root'] and t['va'] == r['va'] and before[0]['_line'] < t['_line'] < r['_line']]
                    if [t['event'] for t in ts] != ['request', 'complete'] or any(ts[0][k] != ts[1][k] for k in ('request', 'targets')):
                        raise ValueError('unmap lacks matching TLB completion')
                    if r['_line'] >= p[1]['_line']:
                        raise ValueError('backing reclaimed before view retired')
                    del live[identity]
            if live or not v or v[0]['event'] != 'published':
                raise ValueError('surviving view or missing publication')
            resources.append({'resource': resource, 'owner': {k:q[0][k] for k in ('pid','incarnation','session','epoch')}, 'frame':p[0]['value'], 'bytes':4096, 'views_retired':sum(r['event']=='after-unmap' for r in v), 'terminal':'PHYSICALLY_RECLAIMED_RECEIPT_CONSUMED'})
        if len(resources) != 13:
            raise ValueError('unexpected test resource inventory')
        for mode in MODES:
            marks = [r for r in user if r['case'] == mode]
            begin, attached, end = (r['_line'] for r in marks)
            subset = [r for r in views if begin < r['_line'] < end]
            publications = [r for r in subset if r['event'] == 'published']
            # One lifecycle backing, then an independent stale-handle control backing.
            if len(publications) != 2:
                raise ValueError('missing per-scenario resource identity')
            resource = publications[0]['resource']
            actor_maps = [r for r in subset if r['resource'] == resource]
            if len([r for r in actor_maps if r['event'] in ('published', 'attached')]) != 2:
                raise ValueError('missing surviving mapping')
            # Child owner in owner cases; attached child in peer cases.
            actor = next(r for r in actor_maps if r['event'] == ('attached' if mode.startswith('r5-peer') else 'published'))
            dead = [r for r in process if r['event']=='reaped' and r['pid']==actor['pid'] and r['incarnation']==actor['incarnation'] and attached < r['_line'] < end]
            code = '137' if mode.endswith('kill') else '132' if mode.endswith('crash') else '0'
            if len(dead) != 1 or dead[0]['exit'] != code:
                raise ValueError('missing actual child reaping/exit status')
            consumed = next(r for r in quota if r['resource']==resource and r['event']=='consumed')
            if consumed['_line'] <= dead[0]['_line']:
                raise ValueError('premature refund before survivor retirement')
            if mode.endswith('crash'):
                fault_lines = [i for i,line in enumerate(raw.splitlines()) if '[FAULT] #6 ' in line and attached < i < dead[0]['_line']]
                if len(fault_lines)!=1 or not attached < fault_lines[0] < dead[0]['_line']:
                    raise ValueError('missing genuine invalid-opcode termination')
                if not any('process_mark_finished pid='+actor['pid']+' ' in line and 'reason=invalid-opcode code=132' in line for line in raw.splitlines()[attached:dead[0]['_line']]):
                    raise ValueError('missing exact locked fault termination identity')
            if mode.endswith('exec'):
                replacements = [r for r in snapshots if r['label']=='exec-replaced' and r['pid']==actor['pid'] and r['incarnation']==actor['incarnation'] and r['vm'] != actor['vm']]
                if len(replacements)!=1:
                    raise ValueError('missing same-incarnation VM replacement')
            scenarios.append({'case':mode,'status':'PASS','resource':int(resource),'actor':{k:actor[k] for k in ('pid','incarnation','vm')},'exit_code':int(code),'terminal':'PHYSICALLY_RECLAIMED_RECEIPT_CONSUMED','ordering':'peer attach < real lifecycle < child reaped < surviving final view removal < PMM free < receipt consumed'})
        inventories = records(raw,'[R5-INVENTORY] ')
        endpoints = [r for r in inventories if r['label'] in ('baseline','quiescent')]
        if len(endpoints)!=2 or any(r[k]!='0' for r in endpoints for k in ('session_views','session_pending_reaps')):
            raise ValueError('session views/deferred reaping did not close')
        reused=[]
        freed={}
        for r in physical:
            if r['event']=='frame-freed': freed[r['value']]=r
            elif r['event']=='backing' and r['value'] in freed:
                old=freed[r['value']]
                reused.append({'frame':r['value'],'old_resource':old['resource'],'new_resource':r['resource'],'free_line':old['_line'],'backing_line':r['_line']})
        reclamation={'test_owned_allocated_frames':len(resources),'test_owned_reclaimed_frames':len(resources),
                     'test_owned_frame_drift':0,'test_owned_receipts_final':0,'test_owned_views_final':0,
                     'retired_views':sum(r['event']=='after-unmap' for r in views),'reuse_witnesses':reused,
                     'peak_process_bytes':max(int(r['process_bytes']) for r in snapshots),
                     'peak_session_bytes':max(int(r['session_bytes']) for r in snapshots),
                     'peak_receipts':max(int(r['live_receipts']) for r in snapshots),
                     'initial':snapshots[0],'final':snapshots[-1],
                     'system_initial':endpoints[0],'system_final':endpoints[1],
                     'system_free_frame_delta':int(snapshots[-1]['free_frames'])-int(snapshots[0]['free_frames']),
                     'system_global_zero_drift_claimed':False}
        if any(marker in raw for marker in ('PANIC','panicked at','checkpoint=FAIL','UNEXPECTED')):
            raise ValueError('guest failure')
    except (ValueError,KeyError,TypeError,StopIteration) as e:
        errors.append(str(e))
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'scope':'six scheduled Account lifecycle scenarios and runtime handle controls only','resources':resources,'scenarios':scenarios,'reclamation':reclamation,'internal_race_orderings':'NOT_RUN','native_identity_forgery':'NOT_RUN'}
