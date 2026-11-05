#!/usr/bin/env python3
"""Reconcile M3.1 observations without changing the M1/M2 census acceptance gate."""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path

from fastlane_mapping_validate import rows

NAMES = {1: 'claim', 2: 'free', 3: 'reclass', 4: 'live', 5: 'table-link',
         6: 'root', 7: 'shm-create', 8: 'shm-add', 9: 'shm-release',
         10: 'shm-retire', 11: 'shm-live', 12: 'view', 13: 'tlb-request',
         14: 'tlb-ack-produced', 15: 'tlb-complete', 16: 'table-live',
         17: 'checkpoint', 18: 'unmap', 19: 'view-add', 20: 'table-retire'}


def assess(raw):
    errors = []
    events = []
    try:
        for row in rows(raw, '[FL-OWN] '):
            if '_duplicate' in row:
                raise ValueError('duplicate event field')
            event = {k: int(row[k]) for k in ('seq', 'kind', *(f'x{i}' for i in range(8)))}
            if any(value < 0 or value > (1 << 64) - 1 for value in event.values()):
                raise ValueError('invalid unsigned trace field')
            if event['kind'] not in NAMES:
                raise ValueError('unknown event kind')
            events.append(event)
        end = rows(raw, '[FL-CENSUS-TRACE-END] ')
        if len(end) != 1 or '_duplicate' in end[0]:
            raise ValueError('missing/duplicate end receipt')
        end = {k: int(end[0][k]) for k in ('reserved', 'emitted', 'incomplete', 'overflow')}
        if end['overflow'] or end['incomplete']:
            errors.append('trace overflow or incomplete publication')
        if end['reserved'] != end['emitted'] or len(events) != end['emitted']:
            errors.append('trace count mismatch')
        if [e['seq'] for e in events] != list(range(len(events))):
            errors.append('event sequence missing, duplicated or reordered')
        censuses = rows(raw, '[FL-CENSUS] ')
        census = {}
        for r in censuses:
            if '_duplicate' in r:
                raise ValueError('duplicate census field')
            parsed = {k: int(r[k]) for k in ('checkpoint', 'tables', 'shm')}
            if min(parsed.values()) < 0:
                raise ValueError('negative census field')
            census[parsed['checkpoint']] = parsed
        if len(census) != len(censuses):
            errors.append('duplicate census checkpoint')
    except (KeyError, ValueError, TypeError) as error:
        return {'status': 'FAIL', 'errors': [str(error)], 'scope': 'diagnostic only'}

    live = {}
    history = defaultdict(list)
    inventory = defaultdict(dict)
    tables = defaultdict(dict)
    objects = defaultdict(dict)
    views = defaultdict(list)
    roots = defaultdict(list)
    schema = max((e["x1"] for e in events if e["kind"] == 17), default=1)
    strict = schema >= 2
    shm_state = {}
    view_state = {}
    last_unmap = {}
    table_links = {}
    table_retired = {}
    tlbs = {}
    pending_unmaps = {}
    checkpoints = []
    for e in events:
        k = e['kind']
        d = [e[f'x{i}'] for i in range(8)]
        if k in (1, 2, 3, 5, 7, 8, 9, 10, 20):
            history[d[0]].append({'event': NAMES[k], **e})
        if k == 1:
            if d[0] in live:
                errors.append(f'duplicate tracked claim {d[0]}')
            live[d[0]] = (d[1], d[2], e['seq'])
            shm_state.pop(d[0], None)
            table_links.pop(d[0], None)
            table_retired.pop(d[0], None)
        elif k == 2:
            obj = shm_state.get(d[0])
            if obj and (obj['refs'] or not obj['releasable']):
                errors.append(f'backing freed before final release {d[0]}')
            if strict and d[1] == 6 and d[0] in table_links and d[0] not in table_retired:
                errors.append(f'table freed without retirement condition {d[0]}')
            old = live.pop(d[0], None)
            if old is None or old[:2] != (d[1], d[2]):
                errors.append(f'unmatched tracked free {d[0]}')
        elif k == 3:
            old = live.pop(d[0], None)
            if d[1] in (6, 8, 15) and (old is None or old[:2] != (d[1], d[2])):
                errors.append(f'unmatched reclass {d[0]}')
            if d[3] in (6, 8, 15):
                live[d[0]] = (d[3], d[2], e['seq'])
        elif k == 4:
            if d[0] in inventory[d[3]]:
                errors.append(f'duplicate inventory frame {d[0]} at {d[3]}')
            inventory[d[3]][d[0]] = {'class': d[1], 'owner_pid': d[2],
                                        'allocation_seq': live.get(d[0], (0, 0, None))[2]}
            if live.get(d[0], ())[:2] != (d[1], d[2]):
                errors.append(f'inventory has no matching claim {d[0]} at {d[3]}')
        elif k == 5 or (k == 6 and not d[3]):
            table_links[d[0]] = e
        elif k == 6 and d[3]:
            roots[(d[3], d[1])].append({'pid': d[2], 'incarnation': d[4]})
        elif k in (7, 8, 9, 10):
            old = shm_state.get(d[0])
            if k == 7:
                if old or d[3] or live.get(d[0], (None,))[0] not in (8, 15):
                    errors.append(f'invalid backing publication {d[0]}')
            else:
                if old is None or (old['object'], old['owner']) != (d[1], d[2]):
                    errors.append(f'SHM transition lacks exact object owner {d[0]}')
                elif k in (8, 9) and old['refs'] + (1 if k == 8 else -1) != d[3]:
                    errors.append(f'SHM reference transition mismatch {d[0]}')
                if strict and k == 9:
                    unmap = last_unmap.get(d[1])
                    if unmap is None or unmap['x7'] != d[3] + 1:
                        errors.append(f'SHM release precedes verified unmap {d[0]}')
            shm_state[d[0]] = {'object': d[1], 'owner': d[2], 'refs': d[3],
                                 'releasable': d[3] == 0 and (k == 9 or (k == 10 and bool(d[4])))}
        elif k == 11:
            objects[d[6]][d[0]] = {'object': d[1], 'owner_pid': d[2], 'references': d[3],
                                    'retired': bool(d[4]), 'restricted': bool(d[5]), 'pages': d[7]}
        elif k == 12:
            views[d[6]].append(d)
        elif k == 19:
            key = (d[0], d[1], d[3], d[4])
            if key in view_state:
                errors.append(f'duplicate live mapping view {key}')
            view_state[key] = d
        elif k == 20:
            table_retired[d[0]] = e
            if d[4] == 3:
                if d[5]:
                    errors.append(f'teardown of active table {d[0]}')
            elif d[4] in (1, 2, 4):
                complete = [t for t in tlbs.values() if t['complete']
                            and t['request']['x1'] == d[1] and t['request']['x2'] == d[2]
                            and t['request']['x3'] == d[3]
                            and live.get(d[0], (0, 0, e['seq']))[2] < t['complete']['seq'] < e['seq']]
                if not complete:
                    errors.append(f'table retirement lacks covering shootdown {d[0]}')
            elif d[4] != 5:
                errors.append(f'unknown table retirement condition {d[0]}')
        elif k == 16:
            tables[d[7]][d[0]] = {'vm': d[1], 'root': d[2], 'parent': d[3], 'va': d[4],
                                    'level': d[5], 'occupied_entries': d[6]}
        elif k == 13:
            if d[0] in tlbs:
                errors.append(f'duplicate TLB request {d[0]}')
            tlbs[d[0]] = {'request': e, 'acks': {}, 'complete': None}
        elif k in (14, 15):
            t = tlbs.get(d[0])
            if t is None:
                errors.append(f'TLB event without request {d[0]}')
                continue
            req = [t['request'][f'x{i}'] for i in range(8)]
            if d[1:5] != req[1:5]:
                errors.append(f'TLB identity/range mismatch {d[0]}')
            if k == 14 and d[6] >= 64:
                errors.append(f'invalid ACK CPU {d[6]}')
                continue
            if k == 14:
                if d[6] in t['acks'] or not (req[5] & (1 << d[6])) or t['complete']:
                    errors.append(f'unexpected/late/duplicate ACK {d[0]} CPU {d[6]}')
                t['acks'][d[6]] = e
            else:
                if t['complete'] or d[5] != req[5] or sum(1 << cpu for cpu in t['acks']) != req[5]:
                    errors.append(f'incomplete/duplicate TLB completion {d[0]}')
                t['complete'] = e
        elif k == 18:
            key = tuple(d[:5])
            if d[5] == 0:
                if key in pending_unmaps:
                    errors.append(f'duplicate unmap request {key}')
                pending_unmaps[key] = e['seq']
            else:
                start = pending_unmaps.pop(key, None)
                completed_pages = {t['request']['x3'] for t in tlbs.values()
                                   if t['complete'] and start is not None
                                   and start < t['request']['seq'] < t['complete']['seq'] < e['seq']
                                   and t['request']['x1'] == d[1] and t['request']['x2'] == d[2]
                                   and t['request']['x4'] == 1}
                last_unmap[d[0]] = e
                if strict and view_state.pop((d[0], d[6], d[1], d[3]), None) is None:
                    errors.append(f'unmap without matching mapping view {key}')
                if start is None or not set(range(d[3], d[3] + d[4], 4096)) <= completed_pages:
                    errors.append(f'unmap lacks synchronous page completions {key}')
        elif k == 17:
            cp = d[0]
            checkpoints.append(cp)
            observed = inventory[cp]
            expected = {f: (v[0], v[1]) for f, v in live.items() if v[0] in (6, 8)}
            if {f: (v['class'], v['owner_pid']) for f, v in observed.items()} != expected:
                errors.append(f'event/inventory mismatch at {cp}')
            if strict:
                actual_views = {tuple(d[:6]) for d in views[cp] if d[7] == 0}
                if actual_views != {tuple(d[:6]) for d in view_state.values()}:
                    errors.append(f'view event/inventory mismatch at {cp}')
                for frame, obj in objects[cp].items():
                    state = shm_state.get(frame)
                    if state is None or (state['object'], state['owner'], state['refs']) != (obj['object'], obj['owner_pid'], obj['references']):
                        errors.append(f'backing object/event mismatch {frame} at {cp}')
            c = census.get(cp)
            counts = Counter(v['class'] for v in observed.values())
            if c is None or counts[6] != int(c['tables']) or counts[8] != int(c['shm']):
                errors.append(f'inventory/accounting mismatch at {cp}')
    if pending_unmaps or any(t['complete'] is None for t in tlbs.values()):
        errors.append('unfinished unmap or shootdown at final checkpoint')
    if any(cp not in checkpoints for cp in (*range(100, 106), 999)):
        errors.append('original lifecycle checkpoint coverage missing')

    baseline = inventory[100]
    report = []
    unresolved = []
    for cp in checkpoints:
        if cp not in (*range(100, 106), 999):
            continue
        current = inventory[cp]
        additions = []
        # Allocation sequence distinguishes reuse of the same physical address.
        added = [f for f, v in current.items() if baseline.get(f) != v]
        removed = [f for f, v in baseline.items() if current.get(f) != v]
        for f in added:
            v = current[f]
            owner = tables[cp].get(f) if v['class'] == 6 else objects[cp].get(f)
            state = 'UNEXPLAINED'
            if owner and v['class'] == 6:
                owner = {**owner, 'processes': roots.get((cp, owner['vm']), [])}
                if owner['occupied_entries'] or owner['level'] == 4:
                    state = 'TABLE_LINKED_AT_CHECKPOINT'
            elif owner:
                refs = [d for d in views[cp] if d[0] == owner['object'] and d[7] == 0]
                owner = {**owner, 'mapping_views': [dict(zip(('object', 'pid', 'incarnation', 'vm', 'va', 'bytes'), d[:6])) for d in refs]}
                if len(refs) == owner['references'] and refs:
                    state = 'SHM_PINNED_AT_CHECKPOINT'
            if state == 'UNEXPLAINED':
                unresolved.append({'checkpoint': cp, 'frame': f, **v, 'owner': owner})
            additions.append({'frame': f, **v, 'state': state, 'owner': owner,
                              'history': history[f]})
        report.append({'checkpoint': cp, 'tables': sum(v['class'] == 6 for v in current.values()),
                       'shm': sum(v['class'] == 8 for v in current.values()),
                       'additions': additions, 'removed_allocations': removed})
    return {'status': 'FAIL' if errors or unresolved else 'RECONCILED_OBSERVATIONS',
            'errors': errors, 'unresolved': unresolved, 'checkpoints': report,
            'tlb_requests': len(tlbs), 'remote_tlb_requests': sum(bool(t['request']['x5']) for t in tlbs.values()),
            'events': len(events), 'schema': schema, 'scope': 'new diagnostic image only; linked ownership is not proof of required retention or original failure cause',
            'original_failure_attribution': 'UNAVAILABLE: original source/image/frame manifests missing'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('serial', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = assess(args.serial.read_text(errors='replace'))
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(result['status'], result['errors'])
    return int(result['status'] == 'FAIL')


if __name__ == '__main__':
    raise SystemExit(main())
