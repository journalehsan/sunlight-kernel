#!/usr/bin/env python3
"""Qualify one isolated-guest forced-revocation order; preserve original scope."""
import re
from fastlane_m53r5_assess import records


def assess(raw, order):
    if order not in ("cleanup-first", "retirement-first"):
        raise ValueError("explicit ordering required")
    lines = raw.splitlines()
    events = []
    for i, line in enumerate(lines):
        if '[R54] ' in line or '[R55] ' in line:
            events.append({'_line': i, **dict(re.findall(r'(\w+)=([^ ]+)', line))})
    errors = []
    if raw.count('[R55-SESSION] event=retired-stale-generation-denied') != 1:
        errors.append('missing retired stale-session-generation rejection')
    attached = [e for e in events if e.get('event') == 'attached']
    held = [e for e in events if e.get('event') == 'retired-owner-held']
    cleanup = [e for e in events if e.get('event') == 'cleanup-first-complete']
    if len(attached) != 1 or {e.get('order') for e in attached} != {order}:
        errors.append('missing two distinct restricted attachments')
    denied = [e for e in events if e.get('event') == 'retired-denied']
    if len(denied) != 1 or any(denied[0].get(k) != 'denied' for k in ('attachment', 'mapping', 'proposal', 'reservation', 'stale_channel')):
        errors.append('missing authorization boundaries')
    if order == 'retirement-first' and len(held) != 1:
        errors.append('missing held-owner boundary')
    # Preserve actual kernel records, independently of the user completion.
    generations = {e.get('generation') for e in attached}
    receipts = []
    for i, line in enumerate(lines):
        fields = dict(re.findall(r'(\w+)=([^ ]+)', line))
        if fields.get('generation') in generations and any(tag in line for tag in ('[FL-BIND]', '[FL-AUTH]', '[FL-MAP]', '[FL-TLB]', '[FL-FRAME]')):
            receipts.append({'_line': i, 'raw': line, **fields})
    lifetimes = []
    for e in attached:
        gen = e['generation']
        maps = [r for r in receipts if '[FL-MAP]' in r['raw'] and r.get('generation') == gen and r.get('event') == 'map']
        frees = [r for r in receipts if '[FL-FRAME]' in r['raw'] and r.get('generation') == gen and r.get('event') == 'free']
        if len(maps) != 2 or len(frees) != 1:
            errors.append('missing exact two mappings/one physical free for generation ' + gen)
        if len(maps) == 2 and ({r.get('role') for r in maps} != {'0', '1'} or len({r.get('frame') for r in maps}) != 1):
            errors.append('mapping roles/frame mismatch for ' + gen)
        # Bind immutable owner/peer identities through independent kernel output.
        binds = [r for r in receipts if '[FL-BIND]' in r['raw'] and r.get('generation') == gen and r.get('op') == '3']
        if len(binds) != 2 or any(r.get('producer') != e['owner'] or r.get('consumer') != e['peer'] for r in binds):
            errors.append('missing exact identity-bound role attachments for ' + gen)
        try:
            def one(rows, **fields):
                found = [r for r in rows if all(r.get(k) == v for k, v in fields.items())]
                if len(found) != 1:
                    raise ValueError('missing/duplicate exact receipt ' + str(fields))
                return found[0]
            backing = one(receipts, generation=gen, event='backing')
            free = one(receipts, generation=gen, event='free')
            physical = records(raw, '[R5-RESOURCE] ')
            reservation = one(records(raw, '[R5-QUOTA] '), event='reserved', pid=e['owner'])
            resource = reservation['resource']
            freed = one(physical, event='frame-freed', resource=resource, value=backing['frame'])
            quotas = [r for r in records(raw, '[R5-QUOTA] ') if r['resource'] == resource]
            if [r['event'] for r in quotas] != ['reserved', 'consumed']:
                raise ValueError('receipt missing or consumed twice')
            reserve, consume = quotas
            if (reserve['pid'] != e['owner'] or reserve['bytes'] != '4096'
                    or any(reserve[k] != consume[k] for k in ('pid', 'incarnation', 'bytes'))
                    or consume['process_after'] != '0'):
                raise ValueError('immutable payer/charge mismatch')
            charged = one(records(raw, '[R5-CHARGE] '), resource=resource)
            if int(charged['process_before']) < 4096 or int(charged['session_before']) < 4096:
                raise ValueError('charge missing before backing reclaim')
            if not reserve['_line'] < backing['_line'] < charged['_line'] < free['_line'] < freed['_line'] < consume['_line']:
                raise ValueError('physical reclaim/refund order')
            views = []
            for mapping in maps:
                key = {k: mapping[k] for k in ('generation', 'region', 'pid', 'inc', 'vm', 'va')}
                before = one(receipts, event='request', **key)
                after = one(receipts, event='complete', **key)
                tlb_key = {k: mapping[k] for k in ('generation', 'region', 'vm', 'va')}
                ts = [r for r in receipts if '[FL-TLB]' in r['raw'] and all(r.get(k) == v for k, v in tlb_key.items())]
                request = one(ts, event='request')
                complete = one(ts, event='complete', request=request['request'], targets=request['targets'])
                if not mapping['_line'] < before['_line'] < request['_line'] < complete['_line'] < after['_line'] < free['_line']:
                    raise ValueError('mapping/TLB retirement before physical reclaim missing')
                targets = int(request['targets'])
                acks = [r for r in ts if r['event'] == 'ack']
                expected = {i for i in range(64) if targets & (1 << i)}
                if {int(r['cpu']) for r in acks} != expected or len(acks) != len(expected):
                    raise ValueError('missing exact target ACK')
                if any(r['request'] != request['request'] or r['observed'] != request['request']
                       or r.get('flushed') != '1' or not request['_line'] < r['_line'] < complete['_line'] for r in acks):
                    raise ValueError('unrelated/late ACK')
                views.append(dict(mapping=mapping, before=before, request=request, acks=acks,
                                  complete=complete, after=after))
            termination = one(records(raw, '[R5-PROCESS] '), event='termination-committed', pid=e['owner'])
            reap = one(records(raw, '[R5-PROCESS] '), event='reaped', pid=e['owner'])
            if e['order'] == 'retirement-first' and not consume['_line'] < termination['_line'] < reap['_line']:
                raise ValueError('required eager retirement diagnostic ordering absent')
            snapshots = records(raw, '[R5-SNAPSHOT] ')
            snap = one(snapshots, label='r54-' + e['order'] + '-attached')
            if (reserve['session'], reserve['epoch']) != (snap['session'], snap['epoch']):
                raise ValueError('session charge owner changed')
            lifetimes.append(dict(order=e['order'], generation=gen, resource=resource,
                                  quota=quotas, charged=charged, backing=backing,
                                  physical_free=freed, views=views, termination=termination, reap=reap))
        except (ValueError, KeyError) as error:
            errors.append(str(error))
    census = [r for r in records(raw, '[FL-CENSUS] ') if r.get('checkpoint') == '540']
    if len(census) != 1 or any(r.get(k) != '0' for r in census for k in ('objects', 'views', 'frames')):
        errors.append('restricted final inventory did not close twice')
    initial = [r for r in records(raw, '[FL-CENSUS] ') if r.get('checkpoint') == '550']
    if len(initial) != 1 or any(initial[0].get(k) != '0' for k in ('objects', 'views', 'frames')):
        errors.append('missing initial restricted inventory')
    try:
        retired = one(records(raw, '[R51-SESSION] '), event='retired', session=reserve['session'], epoch=reserve['epoch'])
        denied_event = one(events, event='retired-denied', generation=gen)
        assert retired['_line'] < denied_event['_line'], 'denials before commit'
        if order == 'cleanup-first':
            cleanup_peer = one(events, event='cleanup-peer-reaped', generation=gen)
            peer_reap = one(records(raw, '[R5-PROCESS] '), event='reaped', pid=e['peer'])
            assert consume['_line'] < reap['_line'] < peer_reap['_line'] < cleanup_peer['_line'] < retired['_line'], 'cleanup-first commits absent'
        else:
            assert retired['_line'] < free['_line'] < denied_event['_line'] < termination['_line'], 'retirement-first commits absent'
        # The survivor must finish ordinary cleanup and have an actual kernel reap.
        ready = one(records(raw, '[R53] '), event='peer-ready')
        survivor_reap = one(records(raw, '[R5-PROCESS] '), event='reaped', pid=ready['peer'])
        final = one(records(raw, '[R5-SNAPSHOT] '), label='retired-final-reclaimed', session_bytes='0')
        assert final['_line'] < survivor_reap['_line'], 'missing terminal inventory/reap'
    except (ValueError, KeyError, AssertionError, UnboundLocalError) as error:
        errors.append(str(error))
    if len(held) == 1:
        prior = [i for i, l in enumerate(lines[:held[0]['_line']]) if '[R51-SESSION] event=retired ' in l]
        frees = [r for r in receipts if r.get('generation') == held[0]['generation'] and '[FL-FRAME]' in r['raw'] and r.get('event') == 'free']
        if not prior or len(frees) != 1 or not prior[-1] < frees[0]['_line'] < held[0]['_line']:
            errors.append('no kernel retirement -> physical free -> held-owner observation')
    return {'diagnostic_status': 'FAIL' if errors else 'PASS', 'status': 'FAIL' if errors else 'PASS', 'original_scenario_status': 'UNQUALIFIED',
            'original_scenario': 'concurrent/session-termination-peer-retention',
            'errors': errors, 'events': events, 'kernel_receipts': receipts,
            'lifetimes': lifetimes, 'final_restricted_inventory': census,
            'missing_witnesses': [], 'session_identity_scope': 'isolated kernel boot; numeric generations are boot-local',
            'scope': 'Independent guest forced-revocation ordering; no original scenario promotion'}
