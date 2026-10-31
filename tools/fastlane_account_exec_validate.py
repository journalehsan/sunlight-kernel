#!/usr/bin/env python3
"""Fail-closed assessment of real authenticated Account exec and one SHM object.

Remote acknowledgements require actual nonzero target masks. Host/parser controls
are not execution evidence; zero targets cannot certify a remote interleaving.
"""
import re


def assess(raw, cpus, fixture, *, identity=None, failure=None, object_id=None):
    if object_id is not None:
        raw = scope_cycle(raw, fixture, str(object_id))
    errors = []
    records = []
    for line in raw.splitlines():
        if line.startswith('[AE-'):
            pairs = re.findall(r'(\w+)=(\S+)', line)
            row = dict(pairs)
            row['_kind'] = line.split()[0]
            if len(row) != len(pairs) + 1:
                errors.append('duplicate trace field')
            records.append(row)
    def require(value, message):
        if not value:
            errors.append(message)
    def rows(kind, event=None):
        return [r for r in records if r['_kind'] == '['+kind+']' and (event is None or r.get('event') == event)]
    def one(kind, event=None):
        found = rows(kind, event)
        require(len(found) == 1, f'exactly one {kind}/{event} required')
        return found[0] if len(found) == 1 else {}
    def n(row, key):
        try:
            return int(row[key])
        except (KeyError, ValueError, TypeError):
            errors.append('missing/invalid '+key)
            return -1
    def before(a, b):
        return n(a, 'seq') < n(b, 'seq')
    def same(a, b, keys):
        return all(a.get(k) is not None and a.get(k) == b.get(k) for k in keys)
    require(not any(s in raw for s in ('PANIC', 'panicked at', 'checkpoint=FAIL')), 'guest fault/assertion')
    g = one('AE-GRANT', 'selected')
    retirement = one('AE-GRANT', 'retired')
    snapshots = {event: one('AE-SNAPSHOT', event) for event in ('committed', 'failed-exec-preserved', 'exec-enter', 'replacement', 'replacement-complete', 'caller-consumed')}
    committed, failed, enter, replacement, done, consumed = (snapshots[e] for e in snapshots)
    worker = one('AE-SNAPSHOT', 'worker-ready')
    held = one('AE-SNAPSHOT', 'worker-held')
    require(same(worker,held,('pid','inc','vm')) and before(enter,held), 'worker did not independently hold caller VM after exec entry')
    controller = one('AE-SNAPSHOT', 'controller-complete')
    require(worker.get('pid') != g.get('caller') and worker.get('vm') == g.get('caller_vm') and before(worker,committed), 'ordinary caller VM borrower did not pretouch before operation')
    for fresh in rows('AE-FRESH'):
        require(fresh.get('pid') == g.get('issuer') and fresh.get('inc') == g.get('issuer_inc') and fresh.get('vm') == replacement.get('vm') and fresh.get('endpoint') == g.get('endpoint') and fresh.get('live') == '1' and fresh.get('generation') != g.get('generation') and before(replacement,fresh), 'fresh call inherited wrong endpoint/process authority')
    fresh_calls = rows('AE-FRESH')
    require(len(fresh_calls) == 2 and {(r.get('caller'), r.get('caller_inc')) for r in fresh_calls} == {(g.get('caller'),g.get('caller_inc')),(controller.get('pid'),controller.get('inc'))}, 'fresh inherited endpoint calls missing/duplicated/wrong incarnation')
    require(all(n(g, k) > 0 for k in ('frame', 'session', 'epoch', 'issuer', 'issuer_inc', 'issuer_vm', 'caller', 'caller_inc', 'caller_vm', 'endpoint', 'generation')), 'invalid grant/binding identity')
    require(g.get('grant') == g.get('object') and n(g,'object') > 0 and g.get('views') == '2', 'selected grant/object or mapping count incorrect')
    for s in (committed, failed, enter):
        require(s.get('auth') == '1' and s.get('pid') == g.get('issuer') and s.get('inc') == g.get('issuer_inc') and s.get('vm') == g.get('issuer_vm'), 'old image is not exact privileged issuer')
    require(committed.get('maps') == '1' and committed.get('deferred') == '0' and enter.get('deferred') == failed.get('deferred') == '1', 'failed exec did not preserve one original deferred target')
    require(before(committed, failed) and before(failed, enter) and before(enter, retirement) and before(retirement, replacement) and before(replacement, done), 'exec/authority retirement order incorrect')
    require(same(enter, replacement, ('pid', 'inc', 'uid', 'gid')) and replacement.get('vm') != enter.get('vm') and n(replacement, 'vm') > 0, 'real replacement identity/VM preservation incorrect')
    require(replacement.get('auth') == '0' and all(replacement.get(k) == '0' for k in ('maps','deferred','pending','outcome','reply')), 'replacement inherited image privilege, mapping or completion')
    require(same(replacement,done,('pid','inc','vm','uid','gid','auth')) and all(done.get(k) == '0' for k in ('pending','outcome','reply','deferred','maps')), 'replacement completion changed identity or retained state')
    require(all(retirement.get(k) == '0' for k in ('sessions', 'receipts', 'grants', 'pending_deliveries')), 'grant/receipt/delivery metadata leaked')
    terminal = one('AE-TERMINAL')
    require(terminal.get('caller') == g.get('caller') and terminal.get('inc') == g.get('caller_inc') and same(terminal,g,('endpoint','generation')) and terminal.get('outcome') == 'PeerClosed('+str(g.get('generation'))+')', 'original caller terminal mismatch')
    require(before(enter, terminal) and before(terminal, replacement) and before(terminal, consumed), 'original terminal ordering incorrect')
    require(consumed.get('pid') == g.get('caller') and consumed.get('inc') == g.get('caller_inc') and consumed.get('vm') == g.get('caller_vm') and all(consumed.get(k) == '0' for k in ('pending','outcome','reply','deferred')), 'caller result not consumed/cleared')
    def snapshot_line(event):
        return next((line for line in raw.splitlines() if line.startswith('[AE-SNAPSHOT]') and 'event='+event+' ' in line), '')
    entry_line, replacement_line = snapshot_line('exec-enter'), snapshot_line('replacement')
    if entry_line and replacement_line:
        window = raw[raw.find(entry_line):raw.find(replacement_line)]
        require(window.count('[SYSCALL] exec path=/bin/session-racer, argc=10,') == 1 and window.count('[SYSCALL] exec: success,') == 1, 'actual exec syscall replacement missing')
    failed_line, committed_line = snapshot_line('failed-exec-preserved'), snapshot_line('committed')
    if failed_line and committed_line:
        require('[SYSCALL] exec: path not found: /missing-account-exec' in raw[raw.find(committed_line):raw.find(failed_line)], 'actual failed exec missing')
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
        expected_views = '2' if m.get('pid') == g.get('issuer') else '1'
        require(request.get('views') == complete.get('views') == expected_views, 'unmap began with incorrect reference count')
        if m.get('pid') == g.get('caller'):
            require(before(terminal,held) and before(held,request), 'caller unmapped before worker hold handshake')
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
    for event in ('worker-cleaned','controller-complete','replacement-stale-and-authority-denied','request-release-and-stale-denied','manager-restored'):
        one('AE-USER',event)
    require(raw.count('case=N6 checkpoint=old-call-peer-closed ') == 1 and raw.count('case=N6 checkpoint=replacement-old-session-denied ') == 1, 'original caller/replacement denials absent')
    require(raw.count('case=N6 checkpoint=fixture-complete ') == 1 and 'fixture='+str(fixture) in raw, 'fixture completion/identity absent')
    for pid in (g.get('issuer'),g.get('caller'),worker.get('pid'),controller.get('pid')):
        require(pid is not None and re.search(r'\[SCHED\] process_reaped pid='+str(pid)+r'\b',raw), 'selected participant not reaped')
    seqs = [n(r,'seq') for r in records if 'seq' in r]
    require(all(s>0 for s in seqs) and len(seqs) == len(set(seqs)) and seqs == sorted(seqs), 'duplicate/reordered trace sequence')
    require(all(0 <= n(r,'cpu') < cpus for r in records if 'cpu' in r), 'invalid trace CPU')
    require(identity is not None and all(identity.get(k) for k in ('iso_matches','elf_matches','services_match','source_matches')), 'source/image identity missing/mismatched')
    require(failure is None, 'driver failed: '+str(failure))
    return {'classification':'FORCED_AND_PASSED' if not errors else 'FORCED_AND_FAILED', 'status':'PASS' if not errors else 'FAIL', 'errors':errors,
            'terminal_protocol':'GUEST_VALIDATED_PENDING_HOST_ACK' if not errors else 'BLOCKED', 'host_terminal_acknowledged':False,
            'fixture':fixture, 'identity':identity, 'grant':g, 'snapshots':snapshots, 'terminal':terminal, 'mapping_releases':releases,
            'logical_grant_retirement':retirement, 'physical_reclamation':physical, 'subsequent_reuse':claims,
            'remote_acknowledgements':remote_acks, 'remote_flushes':sum(r.get('flushed') == '1' for r in rows('AE-TLB','ack-observed')), 'same_object_remote_ack_order':'PASS' if remote_acks and not errors else 'BLOCKED',
            'cpu_participants':sorted({int(r['cpu']) for r in records if r.get('cpu','').isdigit()}),
            'repeated_same_boot_authenticated_cycles':'NOT_RUN', 'scoped_live_resources':{'grants':0 if retirement.get('grants') == '0' else None, 'receipts':0 if retirement.get('receipts') == '0' else None, 'pending_deliveries':0 if retirement.get('pending_deliveries') == '0' else None, 'views':0 if transfer.get('views') == '0' else None, 'frames':0 if physical.get('event') == 'free' else None}}


def scope_cycle(raw, fixture, object_id):
    """Partition by exact boot-local object and actual participant identities.

    Retain the original reaper/boot context. Other fixture results and unrelated
    object events cannot satisfy this cycle's acceptance.
    """
    parsed = [(line, dict(re.findall(r'(\w+)=(\S+)', line))) for line in raw.splitlines()]
    grants = [r for line,r in parsed if line.startswith('[AE-GRANT]') and r.get('event') == 'selected' and r.get('object') == object_id]
    if len(grants) != 1:
        return ''
    g = grants[0]
    participants = {g['issuer'],g['caller']}
    participants.update(r['pid'] for line,r in parsed if line.startswith('[AE-SNAPSHOT]') and r.get('event') == 'worker-ready' and r.get('vm') == g['caller_vm'])
    participants.update(r['caller'] for line,r in parsed if line.startswith('[AE-FRESH]') and r.get('pid') == g['issuer'])
    result = []
    for line,r in parsed:
        if line.startswith('[AE-'):
            if 'object' in r:
                keep = r['object'] == object_id
            elif 'grant' in r:
                keep = r['grant'] == g['grant']
            else:
                keep = r.get('pid') in participants
            if keep:
                result.append(line)
        elif line.startswith('[SESSION-E3-CLIENT]'):
            if r.get('fixture') == str(fixture):
                result.append(line)
        else:
            result.append(line)
    return '\n'.join(result)


def assess_cycles(raw, cpus, fixtures, *, identity=None, failure=None):
    selected = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines() if line.startswith('[AE-GRANT]') and 'event=selected ' in line]
    errors = []
    if len(selected) != len(fixtures) or len(set(fixtures)) != len(fixtures):
        errors.append('complete cycle/fixture count mismatch')
    if len({(r.get('session'),r.get('epoch')) for r in selected}) != len(selected) or len({(r.get('issuer'),r.get('issuer_inc')) for r in selected}) != len(selected):
        errors.append('authentication/session or issuer incarnation reused across cycles')
    ids = [r.get('object') for r in selected]
    if ids != [str(i+1) for i in range(len(fixtures))]:
        errors.append('object correlation identities reused/missing')
    cycles = [assess(raw,cpus,f,identity=identity,failure=failure,object_id=i+1) for i,f in enumerate(fixtures)]
    for i,c in enumerate(cycles):
        errors.extend(f'cycle {i+1}: '+e for e in c['errors'])
    valid = not errors
    return {'classification':'FORCED_AND_PASSED' if valid else 'FORCED_AND_FAILED', 'status':'PASS' if valid else 'FAIL',
            'errors':errors, 'fixture':fixtures[0], 'fixture_ids':fixtures, 'cycles':cycles, 'identity':identity,
            'terminal_protocol':'GUEST_VALIDATED_PENDING_HOST_ACK' if valid else 'BLOCKED', 'host_terminal_acknowledged':False,
            'remote_acknowledgements':sum(c['remote_acknowledgements'] for c in cycles),
            'remote_flushes':sum(c['remote_flushes'] for c in cycles),
            'same_object_remote_ack_order':'PASS' if valid and all(c['remote_acknowledgements'] for c in cycles) else 'BLOCKED',
            'repeated_same_boot_authenticated_cycles':'PASS' if valid and len(cycles)>1 else 'NOT_RUN',
            'scoped_resource_drift':'PASS' if valid and len(cycles)>1 and all(all(v==0 for v in c['scoped_live_resources'].values()) for c in cycles) else 'NOT_RUN',
            'global_allocator_drift':'NOT_RUN'}
