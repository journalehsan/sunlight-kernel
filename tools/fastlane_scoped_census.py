"""Bounded diagnostic trace qualification, separate from fixture acceptance.

No live-service global quiescence is inferred. The ordinary mapping receipt is
complete without the optional PMM trace; once requested, that trace must be
complete. Adversarial evidence models are explicitly distinct from native runs.
"""
from fastlane_channel_validate import rows

CAPACITY = 65536


def assess_trace(raw, required=False):
    events = rows(raw, '[FL-OWN] ')
    checkpoints = rows(raw, '[FL-CENSUS-TRACE] ')
    ends = rows(raw, '[FL-CENSUS-TRACE-END] ')
    requested = required or bool(events or checkpoints or ends)
    errors = []
    if not requested:
        return {'status': 'NOT_REQUESTED', 'errors': [], 'events': 0}
    try:
        if len(ends) != 1 or not checkpoints:
            errors.append('missing/duplicate diagnostic trace closure')
        if any('_duplicate' in r for r in events + checkpoints + ends):
            errors.append('ambiguous diagnostic trace field')
        if len(events) > CAPACITY:
            errors.append('diagnostic trace exceeds bound')
        if [int(e['seq']) for e in events] != list(range(len(events))):
            errors.append('missing/duplicate diagnostic sequence')
        if any(set(e) != {'seq', 'kind', *(f'x{i}' for i in range(8))}
               or not 1 <= int(e['kind']) <= 20
               or any(not 0 <= int(e[f'x{i}']) < 2**64 for i in range(8)) for e in events):
            errors.append('invalid diagnostic event')
        if any(int(c['overflow']) != 0 or int(c['capacity']) != CAPACITY
               or not 0 < int(c['reserved']) <= CAPACITY for c in checkpoints):
            errors.append('diagnostic trace overflow/inconsistent capacity')
        if len(ends) == 1:
            end = ends[0]
            if not (int(end['reserved']) == int(end['emitted']) == len(events)
                    and end['overflow'] == end['incomplete'] == '0'):
                errors.append('incomplete/overflowing diagnostic trace')
        if len([c for c in checkpoints if c.get('checkpoint') == '999']) != 1:
            errors.append('missing diagnostic final checkpoint')
        if checkpoints and ends and checkpoints[-1].get('reserved') != ends[0].get('reserved'):
            errors.append('diagnostic checkpoint/closure mismatch')
        previous = 0
        for c in checkpoints:
            markers = [e for e in events if e['kind']=='17' and e['x0']==c['checkpoint'] and e['x1']=='3']
            reserved = int(c['reserved'])
            if len(markers)!=1 or not previous<=reserved<=len(events) or int(markers[0]['seq'])>=reserved:
                errors.append('diagnostic checkpoint identity/count mismatch')
            previous = reserved
    except (KeyError, ValueError, TypeError):
        errors.append('malformed diagnostic trace')
    return {'status': 'NOT_QUALIFIED' if errors else 'PASS', 'errors': errors,
            'events': len(events), 'closure': ends}


def fixture_trace(raw, objects):
    """Cross-check optional SHM IDs against exact authenticated fixture views.

    The mapping ledger is independently required. Complete diagnostic evidence
    must agree with it; a PMM address by itself never establishes attribution.
    """
    events = rows(raw, '[FL-OWN] ')
    kinds = {str(k): [] for k in range(1,21)}
    for e in events:
        kinds[e['kind']].append(e)
    errors = []
    bound = []
    object_ids = set()

    def need(ok, why):
        if not ok:
            errors.append(why)

    for obj in objects:
        ids = set()
        adds = []
        for view in obj['mappings']:
            m = view['mapping']
            selected = [e for e in kinds['19'] if tuple(e[k] for k in ('x1','x2','x3','x4'))
                        == tuple(m.get(k) for k in ('pid','inc','vm','va')) and e['x5']=='4096']
            need(len(selected)==1, 'diagnostic fixture view identity ambiguous/missing')
            if len(selected)==1:
                ids.add(selected[0]['x0']);adds.extend(selected)
        need(len(ids)==1, 'diagnostic fixture object identity mismatch')
        if len(ids)!=1:
            continue
        oid = next(iter(ids));frame=obj['backing']['frame']
        need(oid not in object_ids, 'diagnostic object reused by another fixture lifetime')
        object_ids.add(oid)
        creates=[e for e in kinds['7'] if e['x1']==oid]
        releases=[e for e in kinds['9'] if e['x1']==oid]
        final=[e for e in releases if e['x3']=='0']
        all_adds=[e for e in kinds['19'] if e['x0']==oid]
        need(len(creates)==len(final)==1 and len(all_adds)==len(adds)==len(releases),
             'diagnostic fixture lifecycle incomplete/duplicate')
        need([int(e['x3']) for e in releases]==list(range(len(adds)-1,-1,-1)),
             'diagnostic fixture view-release inventory mismatch')
        if len(creates)!=1 or len(final)!=1 or not adds:
            continue
        create=creates[0];drop=final[0];owner=min(adds,key=lambda e:int(e['seq']))
        need(create['x0']==frame and create['x2']==owner['x1'] and create['x7']=='1',
             'diagnostic fixture backing owner mismatch')
        need(all(e['x0']==frame and e['x2']==create['x2'] and e['x5']=='1' for e in releases),
             'diagnostic fixture release identity mismatch')
        need(int(create['seq'])<min(int(e['seq']) for e in adds), 'diagnostic fixture publication order')
        for view in obj['mappings']:
            m=view['mapping']
            unmaps=[e for e in kinds['18'] if (e['x0'],e['x1'],e['x3'],e['x6'])
                    == (oid,m.get('vm'),m.get('va'),m.get('pid')) and e['x4']=='4096']
            req=[e for e in unmaps if e['x5']=='0'];done=[e for e in unmaps if e['x5']=='1']
            need(len(unmaps)==2 and len(req)==len(done)==1, 'diagnostic fixture unmap identity mismatch')
            requests=[t for t in view['tlb'] if t.get('event')=='request']
            if len(req)!=1 or len(done)!=1 or len(requests)!=1:
                continue
            publication=[a for a in adds if (a['x1'],a['x2'],a['x3'],a['x4'])
                         == (m.get('pid'),m.get('inc'),m.get('vm'),m.get('va'))]
            need(len(publication)==1 and int(publication[0]['seq'])<int(req[0]['seq']),
                 'diagnostic fixture view publication/unmap order')
            request=requests[0]
            tr=[e for e in kinds['13'] if e['x0']==request.get('request')]
            td=[e for e in kinds['15'] if e['x0']==request.get('request')]
            ta=[e for e in kinds['14'] if e['x0']==request.get('request')]
            need(len(tr)==len(td)==1, 'diagnostic fixture TLB identity ambiguous/missing')
            if len(tr)!=1 or len(td)!=1:
                continue
            t=tr[0];d=td[0]
            need((t['x1'],t['x2'],t['x3'],t['x4'],t['x5'])
                 ==(m.get('vm'),req[0]['x2'],m.get('va'),'1',request.get('targets'))
                 and all(d[k]==t[k] for k in ('x0','x1','x2','x3','x4','x5')),
                 'diagnostic fixture TLB identity mismatch')
            targets={str(cpu) for cpu in range(64) if int(t['x5']) & (1<<cpu)}
            need(len(ta)==len(targets) and {a['x6'] for a in ta}==targets
                 and all(all(a[k]==t[k] for k in ('x0','x1','x2','x3','x4'))
                         and int(t['seq'])<int(a['seq'])<int(d['seq']) for a in ta),
                 'diagnostic fixture remote ACK mismatch/order')
            need(int(req[0]['seq'])<int(t['seq'])<int(d['seq'])<int(done[0]['seq'])<int(drop['seq']),
                 'diagnostic fixture retirement order')
        transitions=[e for e in events if e['kind'] in ('1','2','3') and e['x0']==frame
                     and int(e['seq'])>int(drop['seq'])]
        need(bool(transitions) and transitions[0]['kind']=='2' and transitions[0]['x1']=='8'
             and transitions[0]['x2']==create['x2'], 'diagnostic fixture physical retirement mismatch')
        need(not any(e['kind']=='2' and e['x0']==frame and int(create['seq'])<int(e['seq'])<int(drop['seq']) for e in events)
             and not (len(transitions)>1 and transitions[1]['kind']=='2'),
             'diagnostic fixture premature/duplicate physical retirement')
        bound.append({'lifetime':obj['lifetime'],'shm_object':oid,
                      'owner':{k:owner[k] for k in ('x1','x2','x3')},'frame':frame})
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'bindings':bound}


def foreign_hold(raw):
    """Exact ordinary-SHM owner/view/lifetime proof for the forced hold only."""
    trace = assess_trace(raw, required=True)
    errors = list(trace['errors'])
    events = rows(raw, '[FL-OWN] ')
    users = rows(raw, '[FL-M2] ')
    held = [r for r in users if r.get('case') == 'foreign-shm-held']
    released = [r for r in users if r.get('case') == 'foreign-shm-released']
    witnesses = []
    if len(held) != 1 or len(released) != 1 or any(
            held[0].get(k) != released[0].get(k) for k in ('pid', 'detail')):
        errors.append('missing/ambiguous independent hold actor')
    if not errors:
        pid = held[0]['pid']
        live = [e for e in events if e['kind'] == '11' and e['x2'] == pid
                and e['x6'] == '100' and e['x5'] == '0']
        if len(live) != 1:
            errors.append('independent backing not uniquely live at checkpoint 100')
        else:
            b = live[0]; obj = b['x1']; frame = b['x0']
            views = [e for e in events if e['kind'] == '12' and e['x0'] == obj
                     and e['x1'] == pid and e['x6'] == '100' and e['x7'] == '0']
            creates = [e for e in events if e['kind'] == '7' and e['x1'] == obj and e['x0'] == frame]
            adds = [e for e in events if e['kind'] == '19' and e['x0'] == obj]
            drops = [e for e in events if e['kind'] == '9' and e['x1'] == obj and e['x3'] == '0']
            unmaps = [e for e in events if e['kind'] == '18' and e['x0'] == obj]
            if not (len(views) == len(creates) == len(adds) == len(drops) == 1 and len(unmaps) == 2):
                errors.append('independent lifecycle incomplete/ambiguous')
            else:
                view = views[0]; add = adds[0]
                if not (creates[0]['x2']==b['x2']==drops[0]['x2']==pid
                        and b['x3']=='1' and b['x7']=='1' and b['x4']==b['x5']=='0'
                        and drops[0]['x0']==frame and drops[0]['x5']=='0'):
                    errors.append('independent object owner/lifetime mismatch')
                if any(view[k] != add[k] for k in ('x0','x1','x2','x3','x4','x5')):
                    errors.append('independent view identity mismatch')
                if not all(int(view[k]) > 0 for k in ('x0','x1','x2','x3','x4','x5')):
                    errors.append('independent owner identity missing')
                req = [e for e in unmaps if e['x5'] == '0']
                done = [e for e in unmaps if e['x5'] == '1']
                if len(req) != 1 or len(done) != 1:
                    errors.append('independent unmap missing')
                else:
                    r, d = req[0], done[0]
                    if any(r[k] != d[k] for k in ('x0','x1','x2','x3','x4','x6')) or (
                            r['x1'],r['x3'],r['x4'],r['x6']) != (view['x3'],view['x4'],view['x5'],pid):
                        errors.append('independent unmap identity mismatch')
                    between = [e for e in events if e['kind'] == '13'
                               and e['x1'] == view['x3'] and e['x3'] == view['x4']
                               and int(r['seq']) < int(e['seq']) < int(d['seq'])]
                    tc = []
                    if len(between) != 1:
                        errors.append('independent TLB request missing')
                    else:
                        t = between[0]
                        tc = [e for e in events if e['kind'] == '15' and e['x0'] == t['x0']]
                        if len(tc) != 1 or not int(t['seq']) < int(tc[0]['seq']) < int(d['seq']):
                            errors.append('independent TLB completion missing/order')
                    frees = [e for e in events if e['kind'] == '2' and e['x0'] == frame
                             and int(e['seq']) > int(drops[0]['seq'])]
                    # Take the next frame transition, never a later reuse/free.
                    transitions = [e for e in events if e['kind'] in ('1','2','3') and e['x0'] == frame
                                   and int(e['seq']) > int(drops[0]['seq'])]
                    if not frees or not transitions or transitions[0] != frees[0]:
                        errors.append('independent lifetime physical release missing')
                    elif frees[0]['x1']!='8' or frees[0]['x2']!=pid:
                        errors.append('independent physical release owner/class mismatch')
                    if any(e['kind']=='2' and e['x0']==frame and int(creates[0]['seq'])<int(e['seq'])<int(drops[0]['seq']) for e in events) or (
                            len(transitions)>1 and transitions[1]['kind']=='2'):
                        errors.append('independent premature/duplicate physical retirement')
                    if not int(creates[0]['seq']) < int(add['seq']) < int(b['seq']) < int(r['seq']) < int(d['seq']) < int(drops[0]['seq']):
                        errors.append('independent lifetime ordering')
                    witnesses = [creates[0], add, b, view, *unmaps, *between, *tc,
                                 drops[0], *(frees[:1])]
            if any(e['kind']=='12' and e['x0']==obj and e['x7']=='1' for e in events):
                errors.append('independent allocation attributed to fixture')
            census = rows(raw, '[FL-CENSUS] ')
            cycles = [c for c in census if c.get('checkpoint') in ('100','101')]
            if len(cycles) != 2 or int(cycles[0]['shm']) != int(cycles[1]['shm']) + 1:
                errors.append('controlled global variation missing')
    return {'status': 'FAIL' if errors else 'PASS', 'errors': errors,
            'scope': 'controlled-independent-lifetime-only', 'witnesses': witnesses,
            'historical_attribution': 'UNRESOLVED'}
