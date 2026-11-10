"""Qualify native session retirement/owner cleanup orders from exact identities.

Shared-word releases establish legal syscall-boundary orders, not simultaneous
lock-held execution. Missing, duplicate or inconsistent receipts fail closed.
"""
from fastlane_m53r5_assess import records

SCENARIO = 'concurrent/session-termination-peer-retention'


def one(rows, **fields):
    matches = [r for r in rows if all(r.get(k) == v for k, v in fields.items())]
    if len(matches) != 1:
        raise ValueError('expected one receipt: ' + str(fields))
    return matches[0]


def peer_reaped(raw):
    """Runner completion condition includes actual surviving-peer cleanup."""
    try:
        ready = one(records(raw, '[R53] '), event='peer-ready')
        one(records(raw, '[R5-PROCESS] '), event='reaped', pid=ready['peer'])
        one(records(raw, '[R53] '), event='all-complete')
        return True
    except (ValueError, KeyError):
        return False


def assess(raw, *, restricted_extra=False):
    orders, lifetimes = [], []
    try:
        marks = records(raw, '[R53] ')
        begin = one(marks, event='begin')
        ready = one(marks, event='peer-ready')
        complete = one(marks, event='all-complete')
        orders_done = one(marks, event='orders-complete')
        denied = one(marks, event='owner-retired-denied', allocation='denied', launch='denied')
        snapshots = records(raw, '[R5-SNAPSHOT] ')
        extra = 0
        if restricted_extra:
            # The strict default oracle remains the three-backing workload.
            # Opt-in validates the extra real restricted backing independently.
            if '[R55] ' in raw:
                from fastlane_m53r55_assess import assess as restricted_assess
                audit = restricted_assess(raw, 'retirement-first')
            else:
                from fastlane_m53r54_assess import assess as restricted_assess
                audit = restricted_assess(raw)
            assert audit['diagnostic_status'] == 'PASS', 'extra backing proof failed'
            life = next(x for x in audit['lifetimes'] if x['order'] == 'retirement-first')
            extra = int(life['quota'][0]['bytes'])
            assert extra == 4096, 'unexpected extra topology'
        expected = [('r53-cleanup-first', 8192), ('r53-retirement-first', 12288),
                    ('r53-both-ready', 12288), ('r53-cleanup-before-retirement', 12288),
                    ('retirement-prepared', 12288 + extra), ('r53-retired-admission-denied', 12288),
                    ('r53-owner-retired-denied', 12288), ('r53-retirement-before-cleanup', 12288),
                    ('r53-first-reclaimed', 8192), ('r53-second-reclaimed', 4096),
                    ('retired-surviving-peer', 4096), ('retired-owner-reaped', 4096),
                    ('retired-final-reclaimed', 0)]
        checks = [one(snapshots, label=label, session_bytes=str(n)) for label, n in expected]
        assert [r['_line'] for r in checks] == sorted(r['_line'] for r in checks), 'checkpoint order'
        session, epoch = checks[0]['session'], checks[0]['epoch']
        assert all((r['session'], r['epoch']) == (session, epoch) for r in checks), 'payer migration'
        retired = one(records(raw, '[R51-SESSION] '), event='retired', session=session, epoch=epoch)
        assert retired['outstanding_bytes'] == str(12288 + extra) and retired['registry_present'] == '0', 'early refund'
        if extra:
            q = life['quota'][0]
            assert (q['session'], q['epoch']) == (session, epoch), 'extra backing foreign payer'
            assert checks[2]['_line'] < q['_line'] < checks[4]['_line'] < retired['_line'] < life['quota'][1]['_line'] < checks[5]['_line'], 'extra backing interval'

        assert checks[4]['_line'] < retired['_line'] < checks[5]['_line'], 'retirement boundary'
        assert begin['_line'] < ready['_line'] < checks[3]['_line'], 'missing prepared actors'
        assert denied['_line'] < checks[7]['_line'] < orders_done['_line'] < complete['_line'], 'denial/cleanup order'
        views = records(raw, '[R5-VIEW] ')
        quota = records(raw, '[R5-QUOTA] ')
        physical = records(raw, '[R5-RESOURCE] ')
        process = records(raw, '[R5-PROCESS] ')
        tlbs = records(raw, '[R5-TLB] ')
        peer = ready['peer']
        peer_birth_view = [r for r in views if r['event'] == 'attached' and r['pid'] == peer
                           and r['_line'] < begin['_line']][-1]
        control_resource = peer_birth_view['resource']
        owner_resources = []
        for mode, field in [('r53-cleanup-first', 'before_owner'), ('r53-retirement-first', 'after_owner')]:
            owner_ready = one(marks, event='owner-ready', mode=mode, pid=ready[field])
            owner_view = one([r for r in views if begin['_line'] < r['_line'] < ready['_line']],
                             event='published', pid=ready[field])
            resource = owner_view['resource']
            owner_resources.append(resource)
            attached = one(views, resource=resource, event='attached', pid=peer)
            termination = one(process, event='termination-committed', pid=owner_view['pid'],
                              incarnation=owner_view['incarnation'], vm=owner_view['vm'], session=session, epoch=epoch)
            reaped = one(process, event='reaped', pid=owner_view['pid'],
                         incarnation=owner_view['incarnation'], vm=owner_view['vm'], session=session, epoch=epoch)
            committed = one(physical, event='reference-committed', resource=resource, value=owner_view['pid'])
            assert owner_view['_line'] < owner_ready['_line'] < attached['_line'] < ready['_line'] < termination['_line'], 'actor readiness'
            assert termination['_line'] < committed['_line'] < reaped['_line'] and reaped['exit'] == '0', 'process cleanup'
            assert committed['inventory'] == '1', 'surviving peer lost'
            if mode == 'r53-cleanup-first':
                assert reaped['_line'] < checks[3]['_line'] < retired['_line'], 'wrong cleanup-first winner'
            else:
                assert retired['_line'] < checks[5]['_line'] < checks[6]['_line'] < termination['_line'], 'wrong retirement-first winner'
                assert reaped['_line'] < checks[7]['_line'], 'missing second reap'
            orders.append(dict(mode=mode, status='PASS', resource=resource, owner=owner_view,
                               peer=attached, ready=owner_ready, termination=termination,
                               reference_commit=committed, reaped=reaped, retirement=retired))
        resources = [control_resource, *owner_resources]
        assert len(set(resources)) == 3, 'backing identity reused'
        for resource in resources:
            q = [r for r in quota if r['resource'] == resource]
            assert [r['event'] for r in q] == ['reserved', 'consumed'], 'duplicate/missing receipt'
            assert q[0]['bytes'] == q[1]['bytes'] == '4096', 'wrong backing charge'
            assert (q[0]['session'], q[0]['epoch']) == (session, epoch), 'foreign session payer'
            assert all(q[0][k] == q[1][k] for k in ('pid', 'incarnation')), 'payer identity changed'
            p = [r for r in physical if r['resource'] == resource]
            events = [r for r in p if r['event'] in ('backing', 'before-reclaim', 'frame-freed', 'after-reclaim')]
            assert [r['event'] for r in events] == ['backing', 'before-reclaim', 'frame-freed', 'after-reclaim'], 'physical lifetime'
            assert events[0]['value'] == events[2]['value'], 'frame identity changed'
            assert events[1]['value'] == '1' and int(events[2]['inventory']) == int(events[1]['inventory']) + 1, 'actual PMM free missing'
            charge = one(records(raw, '[R5-CHARGE] '), resource=resource, pid=q[0]['pid'], incarnation=q[0]['incarnation'], bytes='4096')
            assert int(charge['process_before']) >= 4096 and int(charge['session_before']) >= 4096, 'backing uncharged before free'
            assert charge['_line'] < events[1]['_line'] and q[1]['process_after'] == '0', 'payer closure'
            assert retired['_line'] < events[1]['_line'] < events[2]['_line'] < events[3]['_line'] < q[1]['_line'], 'early physical reclaim/refund'
            v = [r for r in views if r['resource'] == resource]
            published = [r for r in v if r['event'] in ('published', 'attached')]
            expected_views = 4 if resource == control_resource else 2
            assert len(published) == expected_views and len(v) == expected_views * 3, 'view duplication/loss'
            assert one(published, event='published')['pid'] == q[0]['pid'], 'foreign backing owner'
            commits = [r for r in p if r['event'] == 'reference-committed']
            assert [int(r['inventory']) for r in commits] == list(reversed(range(expected_views))), 'reference retirement drift'
            retired_views = []
            for view in published:
                identity = {k: view[k] for k in ('pid', 'incarnation', 'vm', 'root', 'va')}
                before = one(v, event='before-unmap', **identity)
                after = one(v, event='after-unmap', **identity)
                commit = one(commits, value=view['pid'])
                assert view['_line'] < before['_line'] < after['_line'] < commit['_line'] < events[1]['_line'], 'live mapping at free'
                request = one([r for r in tlbs if before['_line'] < r['_line'] < after['_line']],
                              event='request', **{k: view[k] for k in ('vm', 'root', 'va')})
                ack = one(tlbs, event='complete', request=request['request'],
                          **{k: request[k] for k in ('vm', 'root', 'va', 'targets')})
                assert request['_line'] < ack['_line'] < after['_line'], 'missing quiescence'
                assert int(ack['flushed']) & ~int(ack['targets']) == 0, 'unrelated ACK'
                retired_views.append(dict(view=view, before=before, after=after, commit=commit,
                                          tlb_request=request, tlb_complete=ack))
            lifetimes.append(dict(resource=resource, charge=q, physical=events, views=retired_views))
        assert not [r for r in quota if r['event'] == 'reserved' and r.get('session') == session
                    and r.get('epoch') == epoch and r['_line'] > retired['_line']], 'post-retirement allocation'
        final = checks[-1]
        zero_denied = one(marks, event='zero-charge-denied', allocation='denied', launch='denied', authority='absent')
        assert final['_line'] < zero_denied['_line'] < complete['_line'], 'authority revived after refund'
        assert all(final[k] == '0' for k in ('process_bytes', 'session_bytes', 'mappings', 'live_receipts')), 'final quota/receipt/mapping drift'
        inventory = one(records(raw, '[R5-INVENTORY] '), label='retired-final-reclaimed', session=session, epoch=epoch)
        assert inventory['session_views'] == inventory['session_pending_reaps'] == '0', 'pending session cleanup'
        peer_dead = one(process, event='reaped', pid=peer, incarnation=final['incarnation'],
                        vm=final['vm'], session=session, epoch=epoch)
        assert complete['_line'] < peer_dead['_line'] and peer_dead['mappings'] == peer_dead['exit'] == '0', 'survivor not reaped'
        shell = one(views, event='published', resource=control_resource)
        shell_dead = one(process, event='reaped', pid=shell['pid'], incarnation=shell['incarnation'], vm=shell['vm'])
        assert retired['_line'] < shell_dead['_line'] < final['_line'], 'managed shell order'
        assert raw.count('[R51-SESSION-USER] event=complete ') == 1, 'missing session survival regression'
        assert '[SL41] event=retirement-fenced\n' in raw, 'missing trusted launch denial'
        assert not any(s in raw for s in ('PANIC', 'panicked at', 'checkpoint=FAIL', 'UNEXPECTED')), 'guest failure'
        return dict(status='PASS', scenario=SCENARIO, orders=orders, lifetimes=lifetimes,
                    retirement=retired, checkpoints=checks, final_inventory=inventory,
                    survivor_reaped=peer_dead, managed_shell_reaped=shell_dead,
                    test_owned_frames_allocated=3, test_owned_frames_reclaimed=3,
                    test_owned_frame_drift=0, session_identity_reuse='NOT_FORCED',
                    restricted_attach='NOT_RUN', simultaneous_kernel_contention='NOT_CLAIMED')
    except (AssertionError, KeyError, ValueError, IndexError) as error:
        return dict(status='UNQUALIFIED', scenario=SCENARIO, errors=[str(error)],
                    orders=orders, lifetimes=lifetimes)
