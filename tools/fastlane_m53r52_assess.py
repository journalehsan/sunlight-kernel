"""Resource-bound exit/unmap ordering qualification; missing evidence fails closed."""
from fastlane_m53r5_assess import records

SCENARIO = 'concurrent/exit-unmap-with-backing'


def assess(raw):
    results = []
    try:
        marks = records(raw, '[R52] ')
        for mode in ('r52-exit-first', 'r52-unmap-first'):
            begin, ready, end = [next(r for r in marks if r.get('mode') == mode and r['event'] == e)
                                 for e in ('begin', 'peer-ready', 'complete')]
            segment = '\n'.join(raw.splitlines()[begin['_line']:end['_line'] + 1])
            views = records(segment, '[R5-VIEW] ')
            owner = next(r for r in views if r['event'] == 'published')
            resource = owner['resource']
            views = [r for r in views if r['resource'] == resource]
            peer = next(r for r in views if r['event'] == 'attached')
            quota = [r for r in records(segment, '[R5-QUOTA] ') if r['resource'] == resource]
            events = [r for r in records(segment, '[R5-RESOURCE] ') if r['resource'] == resource]
            process = [r for r in records(segment, '[R5-PROCESS] ') if r['pid'] == owner['pid'] and r['incarnation'] == owner['incarnation']]
            termination, = [r for r in process if r['event'] == 'termination-committed']
            reaped, = [r for r in process if r['event'] == 'reaped']
            commits = [r for r in events if r['event'] == 'reference-committed']
            assert len(commits) == 2 and [r['inventory'] for r in commits] == ['1', '0'], 'reference retirement not unique'
            peer_commit, = [r for r in commits if r['value'] == peer['pid']]
            owner_commit, = [r for r in commits if r['value'] == owner['pid']]
            assert (termination['_line'] < peer_commit['_line']) == (mode == 'r52-exit-first'), 'wrong observed winner'
            assert (owner_commit['_line'] < peer_commit['_line']) == (mode == 'r52-exit-first'), 'wrong cleanup order'
            assert [r['event'] for r in quota] == ['reserved', 'consumed'], 'receipt duplication/loss'
            physical = [r for r in events if r['event'] in ('backing', 'before-reclaim', 'frame-freed', 'after-reclaim')]
            assert [r['event'] for r in physical] == ['backing', 'before-reclaim', 'frame-freed', 'after-reclaim'], 'physical lifetime'
            assert physical[0]['value'] == physical[2]['value'], 'frame identity'
            assert commits[-1]['_line'] < physical[1]['_line'] < physical[2]['_line'] < quota[-1]['_line'], 'early free/refund'
            for actor in (owner, peer):
                retired, = [r for r in views if r['event'] == 'after-unmap' and r['pid'] == actor['pid'] and r['vm'] == actor['vm']]
                assert retired['_line'] < physical[1]['_line'], 'live view at free'
            snapshots = records(segment, '[R5-SNAPSHOT] ')
            assert all(any(r['label'] == label and r['session_bytes'] == '8192' for r in snapshots)
                       for label in ('r52-owner-ready', 'r52-peer-ready')), 'both boundary receipts required'
            assert snapshots[-1]['label'] == 'r52-resource-retired' and snapshots[-1]['session_bytes'] == '4096', 'resource charge drift'
            assert owner['pid'] == ready['owner'] == end['owner'] and reaped['exit'] == '0', 'actor identity'
            results.append(dict(mode=mode, status='PASS', resource=resource, owner=owner, peer=peer,
                                receipt=quota, synchronization=snapshots, linearization=[termination, *commits],
                                physical=physical, process=process, views=views, terminal='RECLAIMED_ONCE'))
        final, = [r for r in records(raw, '[R5-SNAPSHOT] ') if r['label'] == 'r52-quiescent']
        assert all(final[k] == '0' for k in ('process_bytes', 'session_bytes', 'live_receipts', 'mappings')), 'final drift'
        assert raw.count('[R52] event=all-complete') == 1, 'missing completion'
        assert not any(s in raw for s in ('PANIC', 'panicked at', 'checkpoint=FAIL')), 'guest failure'
        return dict(status='PASS', scenario=SCENARIO, orders=results, final_inventory=final)
    except (AssertionError, KeyError, ValueError, StopIteration) as error:
        return dict(status='UNQUALIFIED', scenario=SCENARIO, errors=[str(error)], orders=results)
