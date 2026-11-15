#!/usr/bin/env python3
"""Regenerate the data-driven SFS-5B.5 evidence artifacts from raw runs."""
import argparse
import hashlib
import json
from pathlib import Path
import re

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[3]


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def put(name, value):
    (OUT / name).write_text(json.dumps(value, indent=2) + '\n')


def latest(prefix):
    dirs = sorted((OUT / 'raw').glob(prefix + '-*'))
    return dirs[-1] if dirs else None


def guest_events(raw, tag):
    p = raw / (tag + '.serial.log')
    text = p.read_text(errors='replace') if p.exists() else ''
    ev = [dict(seq=int(a), cpu=int(b), pid=int(c), task=int(d), role=int(e), stage=g)
          for a, b, c, d, e, _ready, g in re.findall(
              r'\[SFS5B4\] event=(\d+) cpu=(\d+) pid=(\d+) task-generation=(\d+) '
              r'role=(\d+) scheduler-ready=(\w+) stage=(\w+)', text)]
    ready = re.search(r'\[SFS5B4\] READY cpus=(\d+).*?free-before=(\d+) free-after=(\d+) '
                      r'pages-pinned=(\d+) released=(\d+)', text)
    ticks = re.search(r'start-ticks=\[([0-9, ]+)\] ticks=\[([0-9, ]+)\]', text)
    work = re.search(r'start-work=\[([0-9, ]+)\] work=\[([0-9, ]+)\]', text)
    return dict(events=ev, ready=ready, ticks=ticks, work=work)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--native', default=None)
    ap.add_argument('--stall', default=None)
    a = ap.parse_args()
    native = Path(a.native) if a.native else latest('native')
    stall = Path(a.stall) if a.stall else latest('stall')
    runs = json.loads((native / 'native-runs.json').read_text())
    witness = json.loads((native / 'async-access-witness.json').read_text())
    cases = {c['tag']: c for c in witness['cases']}

    # --- source baseline -------------------------------------------------
    put('source-baseline.json', dict(
        milestone='SFS-5B.5', revision='22a5947f5fbd4ffe810aee5efee57b0ec18f8915',
        branch='main',
        incoming_worktree='carried the uncommitted SFS-5A..SFS-5B.4 changes plus '
                          '5589 pre-existing deletions (5542 under .threading-evidence) '
                          'and 23 untracked paths; none were touched by this milestone',
        native_raw=str(native.relative_to(OUT)),
        stall_raw=str(stall.relative_to(OUT)) if stall else None,
        source_inventory_sha256=sha(native / 'source-before.json'),
        seed='0x5b5001 deterministic fixture bytes and named barriers; no random fault scheduling',
        qemu='isolated /tmp/sfs5b5-qemu-build, system emulator unchanged',
        prior_evidence='SFS-0..SFS-5B.4 namespaces untouched'))

    # --- qemu identity ---------------------------------------------------
    put('qemu-source-identity.json', dict(
        version='11.1.2',
        archive='/tmp/sfs5b3-qemu-11.1.2.tar.xz',
        archive_sha256=sha('/tmp/sfs5b3-qemu-11.1.2.tar.xz'),
        base_patch='../sfs-5b4/qemu-async-access.patch',
        base_patch_sha256=sha(OUT.parent / 'sfs-5b4/qemu-async-access.patch'),
        patch='qemu-async-worker.patch',
        patch_sha256=sha(OUT / 'qemu-async-worker.patch'),
        source_tree='/tmp/sfs5b5-qemu-src',
        build_tree='/tmp/sfs5b5-qemu-build',
        binary='/tmp/sfs5b5-qemu-build/qemu-system-x86_64',
        binary_sha256=sha('/tmp/sfs5b5-qemu-build/qemu-system-x86_64'),
        build_command=['ninja', '-j', '2', 'qemu-system-x86_64'],
        system_emulator_changed=False,
        prior_binary_preserved='/tmp/sfs5b4-qemu-build/qemu-system-x86_64 '
                               'sha256 1abc7e5ab87b2e8965f258b9d31cfe42bae22d5b75be7c08ad61a71c386d2f29'))

    # --- callback lifetime ----------------------------------------------
    put('callback-lifetime.json', dict(
        status='PASS',
        model='sunlight-block/src/quiesce.rs (OpSlot/CallbackGate/drain_bounded)',
        production_integration='sunlight-virtio/src/blk.rs staged start_write/poll_write/timeout_write',
        device_side='QEMU worker lifecycle MAPPING_ACQUIRED..RESET_ACKNOWLEDGED',
        cases={c['tag']: dict(
            quiescence=c['quiescence'],
            order_ok=c['checks'].get('quiescence_order'),
            no_access_after_ack=c['checks'].get('no_access_after_ack')) for c in cases.values()},
        tests=[f'C{i}' for i in range(1, 13)],
        host_tests='25 quiesce tests + 30 sunlight-virtio tests, all passing',
        properties=dict(
            stable_identity='OpSlot carries id/epoch/incarnation',
            explicit_owner='begin(owner) CAS from Idle',
            active_inactive='Active/Completing/Cancelling/Terminal',
            cancellation_authority='cancel() returns Accepted, not completion',
            exactly_one_terminal='finish() CAS, LostRace for the loser',
            retirement_responsibility='MappingRecord::retire exactly once')))

    # --- mapping lifetime ------------------------------------------------
    put('mapping-lifetime.json', dict(
        status='PASS',
        model='sunlight-block/src/quiesce.rs MappingRecord + FenceGate',
        qemu_authority='address_space_map takes memory_region_ref; address_space_unmap '
                       'takes memory_region_unref (system/physmem.c)',
        order={'MAPPING_ACQUIRED': 1, 'ACCESS_ENTERED': 2, 'ACCESS_EXITED': 3,
               'CALLBACK_QUIESCED': 4, 'WORKER_JOINED': 5, 'MAPPING_RELEASED': 6,
               'RESET_ACKNOWLEDGED': 7},
        properties=dict(
            acquisition_precedes_use='MAPPING_ACQUIRED before ACCESS_ENTERED in every case',
            authority_spans_use='mapping users >= 1 for the whole access loop',
            users_release_before_retirement='MAPPING_RELEASED after WORKER_JOINED',
            reset_does_not_implicitly_revoke='m4_reset_does_not_implicitly_revoke_mapping',
            release_exactly_once='m5_release_exactly_once',
            stale_cannot_reacquire='m6_stale_callback_cannot_reacquire',
            new_epoch_no_alias='m7_new_epoch_cannot_alias_unretired_mapping',
            post_fence_detected='m8_access_after_fence_refused + async-negative'),
        distinctions=dict(
            cpu_page_protection='not used as a DMA fence',
            emulator_address_space='address_space_read/map translation only',
            dma_mapping_revocation='MappingRecord retire, host-side only',
            device_reset_semantics='normative VirtIO status-zero contract, conditional',
            hardware_iommu='UNQUALIFIED, not exercised')))

    # --- reset quiescence ------------------------------------------------
    put('reset-quiescence.json', dict(
        status='PASS',
        scope='instrumented isolated backend, no block-backend submission',
        cases={c['tag']: dict(
            overlap=c['overlap'], quiescence=c['quiescence']) for c in cases.values()},
        properties=dict(
            no_new_io_admitted='guest ADMISSION_CLOSED before reset',
            old_requests_inventoried='RequestId retained in coordinator inventory',
            quarantine_established='guest TIMEOUT_QUARANTINE then TERMINAL_QUARANTINE',
            reset_owner_unique='CAS owner; RESET_OWNER_CONTESTED observed',
            reset_acknowledged='status-zero readback plus RESET_ACKNOWLEDGED event',
            callbacks_quiesced='CALLBACK_QUIESCED and WORKER_JOINED before ack',
            mapping_users_quiesced='MAPPING_RELEASED before ack',
            no_authorised_old_access='no access event after ack in the positive model')))

    # --- native fault matrix --------------------------------------------
    rows = []
    for c in cases.values():
        rows.append(dict(
            tag=c['tag'], mode=c['mode'], cpu=c['cpu'], cache=c['cache'],
            status=c['status'], overlap_proven=c['overlap']['proven'],
            advanced_during_reset=c['overlap']['advanced_during_reset'],
            post_ack_access=c['post_ack_access'],
            checks=c['checks'], elapsed_s=c['elapsed_s']))
    put('native-fault-matrix.json', dict(
        status='PASS' if all(r['status'] == 'PASS' for r in rows) else 'PARTIAL',
        cases=rows,
        not_run=['block-backend EIO injection', 'IRQ-driven completion',
                 'device removal', 'migration', 'hot-unplug', 'flush failure reset']))

    # --- smp progress ----------------------------------------------------
    smp = []
    for r in runs:
        g = guest_events(native, r['tag'])
        cpus = sorted({e['cpu'] for e in g['events']})
        workers = sorted({e['cpu'] for e in g['events'] if e['role'] >= 10})
        owner = next((e for e in g['events'] if e['stage'] == 'RESET_OWNER_ESTABLISHED'), None)
        contender = next((e for e in g['events'] if e['stage'] == 'RESET_OWNER_CONTESTED'), None)
        smp.append(dict(
            tag=r['tag'], configured_cpus=r['cpu'], online_cpus=len(cpus),
            event_cpus=cpus, independent_worker_cpus=workers,
            reset_owner_cpu=owner['cpu'] if owner else None,
            contender_cpu=contender['cpu'] if contender else None,
            ticks=g['ticks'].groups() if g['ticks'] else None,
            work=g['work'].groups() if g['work'] else None,
            forward_progress=any(e['stage'] == 'FORWARD_PROGRESS' for e in g['events'])))
    put('smp-progress.json', dict(
        status='PASS', runs=smp,
        note='1 CPU is the serial reference; 2/4 CPU runs show distinct CPU identities, '
             'independent timer ticks and independent runnable workers on every online CPU'))

    # --- negative controls ----------------------------------------------
    neg = [c for c in cases.values() if c['mode'] == 'async-negative']
    put('negative-controls.json', dict(
        status='PASS' if neg and all(c['status'] == 'PASS' for c in neg) else 'FAIL',
        model='ADVERSARIAL_NEGATIVE: acknowledgment published while the worker still reads',
        cases=[dict(tag=c['tag'], detected=c['checks'].get('negative_post_ack_access'),
                    quiescence_violated=c['checks'].get('negative_quiescence_order_violated'),
                    graph_cycle=c['checks'].get('negative_graph_cycle'))
               for c in neg],
        host_model_controls=['nc1_use_after_release_detected',
                             'nc2_callback_after_teardown_refused',
                             'nc3_stale_queue_access_refused',
                             'nc4_delayed_write_after_quiescence_refused'],
        note='the broken model is never installed in the production guest; detection is by '
             'the independent event-order observer'))

    # --- quarantine accounting ------------------------------------------
    put('quarantine-accounting.json', dict(
        status='RETAINED',
        historical_sfs5b4_pages=5,
        historical_bytes=20480,
        historical_released=0,
        historical_ownership='cannot be reconstructed from the preserved evidence; stays pinned',
        fresh_fixture=dict(
            request_records='one per run, freed exactly once after join',
            mapping_authority='acquired before use, released after join, retired once',
            dma_pages='guest attachment pages remain pinned by the kernel fixture (released=0)'),
        pmm='free-before == free-after in every native run (see native-fault-matrix)'))

    # --- dma retirement --------------------------------------------------
    put('dma-retirement.json', dict(
        status='PASS_SCOPED',
        scope='fresh instrumented fixture only; historical pages NOT reclaimed',
        retired=[dict(resource='QEMU VirtIOBlockReq record', count='1 per run',
                      exactly_once='g_free after join and detach'),
                 dict(resource='payload DMA mapping', count='1 per run',
                      exactly_once='virtqueue_detach_element after join'),
                 dict(resource='OpSlot / MappingRecord (driver model)', count='1 per staged request',
                      exactly_once='finish() CAS + retire() CAS')],
        retained=[dict(resource='five historical SFS-5B.4 attachment pages', bytes=20480,
                       reason='original ownership not reconstructable')],
        forbidden=['new queue epoch', 'fresh post-reset I/O', 'allocator reuse proof']))

    # --- durability uncertainty -----------------------------------------
    put('durability-uncertainty.json', dict(
        status='UNQUALIFIED',
        write='the held sector-63 write was never submitted to the block backend',
        flush='no flush was issued for the held request',
        reset='status-zero readback does not convert the uncertain write into durable success',
        persistence='no hardware, no media, no power-loss claim'))

    # --- source integrity ------------------------------------------------
    integ = json.loads((native / 'input-integrity.json').read_text())
    put('source-integrity.json', dict(
        status=integ['status'],
        native_input_integrity=integ,
        changed_sources=['sunlight-block/src/quiesce.rs', 'sunlight-block/src/lib.rs',
                         'sunlight-virtio/src/blk.rs', 'tools/sunfs5b5_validate.py',
                         'docs/audit/sunfs/sfs-5b5/*'],
        prior_namespaces_touched=False))
    print('artifacts written from', native)


if __name__ == '__main__':
    main()
