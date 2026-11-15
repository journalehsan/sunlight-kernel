#!/usr/bin/env python3
"""Derive the milestone ledger from retained runs; never invent missing runs."""
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(__file__).resolve().parent


def write(name, value):
    (OUT/name).write_text(json.dumps(value, indent=2)+'\n')


def latest(pattern):
    paths = sorted((OUT/'raw').glob(pattern))
    if not paths:
        raise RuntimeError(f'No completed evidence: {pattern}')
    return paths[-1]


def main():
    native_path = latest('native-*/native-result.json')
    native = json.loads(native_path.read_text())
    assert len(native['boots']) == 18, 'native matrix incomplete'
    assert all(row['status'] == 'PASS' for row in native['boots'])
    hosted_path = latest('hosted-*/hosted-result.json')
    hosted = json.loads(hosted_path.read_text())
    checks_path = latest('checks-*/checks-result.json')
    checks = json.loads(checks_path.read_text())
    assert all(row['returncode'] == 0 for row in hosted+checks)
    inputs = []
    for path in [native_path, hosted_path, checks_path]:
        integrity = json.loads((path.parent/'input-integrity.json').read_text())
        assert integrity['status'] == 'PASS'
        inputs.append(str((path.parent/'input-integrity.json').relative_to(OUT)))
    holds = [r for r in native['boots'] if r['mode'] == 'hold']
    features = []
    witnesses = []
    accounting = []
    qemu_identity = json.loads((OUT/'qemu-source-identity.json').read_text())
    execution_inputs = json.loads((native_path.parent/'source-before.json').read_text())
    for row in native['boots']:
        invocation = json.loads((native_path.parent/row['serial'].replace('.serial.log','.invocation.json')).read_text())
        assert invocation['source_sha256'] == execution_inputs
        if row['mode'] != 'unmodified':
            assert invocation['qemu_sha256'] == qemu_identity['binary_sha256']
            assert invocation['patch_sha256'] == qemu_identity['patch_sha256']
        serial_path = native_path.parent/row['serial']
        serial = serial_path.read_text()
        match = re.search(r'features=\((\d+), (\d+)\)', serial)
        features.append(dict(cpu=row['cpu'], cache=row['cache'], mode=row['mode'],
                             offered=hex(int(match[1])), accepted=hex(int(match[2]))))
        if row['mode'] != 'hold':
            continue
        lines = [s for s in serial.splitlines() if '[SFS5B3]' in s]
        trace = (native_path.parent/row['trace']).read_text()
        request = re.findall(r'virtio_blk_handle_write vdev (\S+) req (\S+) sector 63 nsectors 1', trace)
        assert len(request) == 1
        dev, req = request[0]
        tail = trace[trace.index(f'virtio_blk_handle_write vdev {dev} req {req} sector 63'):]
        completion_published = f'virtio_blk_req_complete vdev {dev} req {req}' in tail or f'elem {req} len' in tail
        assert not completion_published
        assert row['all_configured_cpu_timer_progress']
        device_lines = (native_path.parent/row['device_log']).read_text().splitlines()
        assert any('SFS5B3 guard-installed' in line and 'probe=EFAULT' in line for line in device_lines), 'missing independent guard negative probe'
        match = re.search(r'free-before=(\d+) free-after=(\d+)', serial)
        accounting.append(dict(cpu=row['cpu'], cache=row['cache'], before=int(match[1]), after=int(match[2]),
                               same_free_count=match[1] == match[2], pinned_pages=5, pinned_bytes=20480,
                               released_pages=0, logical_uncertain_request_retirements=0,
                               scope='PMM global count sample, no individual frame release or reuse proof'))
        witnesses.append(dict(cpu=row['cpu'], cache=row['cache'], guest=lines,
                              device=device_lines, qemu_device=dev, qemu_request=req,
                              used_publication_for_held_request=completion_published,
                              serial=str(serial_path.relative_to(OUT)), trace=str((native_path.parent/row['trace']).relative_to(OUT)),
                              dma_bytes_consumed=256, remaining_bytes=256, backend_submitted=False,
                              guest_request_generation=2, queue_incarnation=1, descriptor_head=0,
                              simultaneous_memory_instruction_at_reset='UNQUALIFIED'))
    write('instrumented-backend.json', dict(status='PASS', scope='test-only modified QEMU 11.1.2 x86_64 TCG, main AioContext, BQL, ioeventfd=off, no IOThread',
          guard='PROT_NONE old four-page queue and one-page request mappings after generic reset; nonblocking pipe copy returns EFAULT on all five pages after a successful payload probe before protection',
          design='Split actual 512-byte sector63 OUT payload consumption at 256 bytes; hold real virtqueue element before backend; timer release consumes rest and submits same data; reset cancels timer and detaches element',
          source='qemu-source-identity.json', patch='qemu-active-io.patch', rebuild='rebuild-qemu.sh',
          timer_only_completion_delay=False, system_installation_modified=False,
          stages={'accepted':'virtqueue_pop trace','fetched':'handle_write and test fetched event','dma_read':'real qemu_iovec_to_buf of guest data mapping','backend_pending':'absent in held case, submitted/completed in release control','completion_prepared':'normal req_complete trace in control only','used_published':'virtqueue_fill/flush in controls only','interrupt':'normal virtio_notify source path; delivery not independently instrumented'},
          features=features))
    write('active-io-witness.json', dict(status='PASS', scope='actual outstanding real virtqueue requests at partial data consumption boundary, before backend submission', witnesses=witnesses))
    write('dma-access-witness.json', dict(status='PARTIAL', actual_device_memory_read='PASS', active_dma_instruction_during_reset='UNQUALIFIED',
          witness_A='native driver token, ownership, timeout, status-zero readback and refusal',
          witness_B='separate QEMU virtqueue trace and actual payload copy instrumentation, plus whole backing-image comparison',
          observed_relation='256-byte payload read -> held request -> guest timeout -> reset entry -> timer cancellation and detach -> queues cleared/status zero; no subsequent instrumented copy',
          post_fence_global_memory_guard='PARTIAL: PROT_NONE original mappings of all five old pages; EFAULT kernel negative probe per held-request reset', observation_window_after_ready_seconds=0.5,
          limits=['transfer paused between memory-copy instructions during reset; not simultaneous DMA execution', 'no asynchronous DMA worker or physical bus transaction drain witness', 'original host mappings protected; aliases/remapping and a hostile hypervisor are not covered', 'test reset cancellation is a model modification, not unmodified QEMU behavior', 'guest= in QEMU custom log is a mapped host pointer, not a reported GPA'],
          witnesses='active-io-witness.json'))
    write('reset-lifecycle.json', dict(status='PARTIAL', native='RESET_ACKNOWLEDGED with one status read in all six held-request cases',
          unsupported='queue reset unavailable; whole-device reset supported', delayed_and_withheld_native='NOT_RUN',
          hosted='64-read delayed/withheld/unexpected/idempotence fixtures rerun in storage regression',
          dma='DMA_QUIESCENCE_UNQUALIFIED', terminal='TERMINAL_QUARANTINE',
          bounds={'submission_polls':50000000,'reset_status_reads':64,'native_ack_reads':1},
          driver_production_changed=False, evidence='active-io-witness.json'))
    write('queue-epoch-closure.json', dict(status='UNQUALIFIED', old_queue_incarnation=1,
          admission='permanently closed after uncertainty/reset', old_outstanding_request='quarantined with generation2/head0',
          old_authority_invalidated_for_new_epoch=False, new_epoch_admission=False,
          reinitialization='NOT_RUN', fresh_post_reset_io='NOT_RUN', reason='No qualified retirement/reuse authority; preserve old ownership'))
    write('dma-retirement.json', dict(status='UNQUALIFIED', released_pages=0, released_bytes=0,
          transferred_mappings=0, old_epoch_request_retirements=0,
          resources={'queue_descriptor_avail_used_region':{'pages':4,'owner':'old attachment, pinned'},
                     'header_bounce_status':{'pages':1,'owner':'old attachment, pinned'},
                     'request_record':{'count':1,'owner':'quarantined driver, retained'}},
          device_model_request_detached=True, distinction='Device-side element freeing does not free guest PMM pages',
          allocator_reclamation_proof='NOT_RUN', accounting='quarantine-accounting.json'))
    write('quarantine-accounting.json', dict(status='PARTIAL', attachments_per_vm=1, pinned_pages_per_attachment=5,
          pinned_bytes_per_attachment=20480, native_samples=accounting,
          requests_per_serialized_attachment=1, quarantine_capacity='one outstanding request; no subsequent admission',
          repeat_reset='one write, no budget restart or new allocation; hosted repeated1024 refusals',
          pressure_limit='no bounded global quarantine manager or attachment-retry allocator budget implemented',
          exhaustion='existing initialization allocation expect may halt on PMM exhaustion; no infinite recovery loop or release introduced',
          native_pressure_test='NOT_RUN'))
    # These are case-specific claims, not promotions of hosted ring injection to real DMA.
    definitions = [
      ('H1','Active request held before completion','PASS','native partial-consumption boundary','active-io-witness.json'),
      ('H2','Timeout while device request outstanding','PASS','native Deadline receipt while QEMU holds request','active-io-witness.json'),
      ('H3','Reset while device processing active','PARTIAL','outstanding partial transfer, paused between reads','active-io-witness.json'),
      ('H4','Reset while memory read/write in progress','UNQUALIFIED','no copy instruction overlapping reset',None),
      ('H5','Reset acknowledgment delayed','PARTIAL','host status fixture only','f10_delayed_ack_has_exact_read_bound'),
      ('H6','Reset acknowledgment withheld','PARTIAL','host status fixture only','f11_withheld_ack_is_terminal_and_not_a_fence'),
      ('H7','Device activity after invalid fence','PARTIAL','host late ring writes before ack; requested invalid-fence device interleaving absent','f3_f8_late_ring_write_and_stale_notification_do_not_reopen_reset'),
      ('H8','Late completion following timeout','PARTIAL','host fake-ring writes, no native late publication','late_completion_must_not_authorize_new_request'),
      ('H9','Old completion after new epoch','NOT_RUN','no new epoch',None),
      ('H10','Duplicate completion','PARTIAL','host fake-ring injection only','f4_head_only_duplicate_cannot_supply_new_status'),
      ('H11','Invalid descriptor head','PARTIAL','host fake-ring injection only','t15_invalid_used_head_and_index'),
      ('H12','Reset racing completion publication','NOT_RUN','interleaving not forced',None),
      ('H13','Reset racing interrupt handling','NOT_RUN','driver polls; independent interrupt race absent',None),
      ('H14','Two reset initiators','PARTIAL','serial repeated begin only; no concurrent owners','f15_f17_f18_reset_competitors_and_pressure_preserve_first_owner'),
      ('H15','Device removal during pending I/O','PARTIAL','host 0xff register only; no unplug','f13_f16_unexpected_or_removed_device_cannot_ack'),
      ('H16','Multiple quarantined requests','NOT_RUN','one serialized outstanding request per attachment',None),
      ('H17','Resource pressure after timeout','PARTIAL','1024 serial refusals; no native allocator pressure','f15_f17_f18_reset_competitors_and_pressure_preserve_first_owner'),
      ('H18','Fresh I/O after verified epoch transition','NOT_RUN','release control is same pre-reset epoch',None),
      ('H19','Failed flush before reset','PARTIAL','host status1/2 only','f21_f22_device_error_provenance_survives_reset_and_refusal'),
      ('H20','Uncertain write before reset','PASS','native partial-consumption write, no durability promotion','active-io-witness.json'),
      ('H21','Repeated bounded recovery cycles','PARTIAL','idempotent terminal calls only; no recovery cycles','f15_f17_f18_reset_competitors_and_pressure_preserve_first_owner'),
      ('H22','Counter/index wraparound','PARTIAL','host ring wrap and generation exhaustion; no native wrap','ring_index_wrap_is_separate_from_generation_exhaustion'),
    ]
    matrix=[]
    for case, title, status, scope, reproducer in definitions:
        is_native = reproducer == 'active-io-witness.json'
        if reproducer and not is_native:
            assert reproducer+' ... ok' in (hosted_path.parent/'storage.log').read_text(), reproducer
        matrix.append(dict(case=case, title=title, status=status, scope=scope,
             initial_ownership='native generation2/head0 queue1, five attachment pages' if is_native else ('see exact named fixture setup' if reproducer else 'NOT_RUN'),
             injected_event=scope, device_visible='real held sector63 element,256/512 payload bytes consumed' if is_native else 'fake ring/register only; actual device state unobserved',
             driver_visible='Deadline Uncertain, Quarantined, then VerifiedRetained' if is_native else 'asserted by named fixture only' if reproducer else 'NOT_RUN',
             expected_safe_outcome='retain resources, refuse admission, preserve uncertainty',
             actual_outcome=scope if reproducer else 'NOT_RUN', dma_page_retirement_count=0,
             independent_witness='QEMU trace and payload copy plus backing-image comparison' if is_native else 'NONE: host injection is not independent device evidence',
             reproducer=reproducer, hosted_command='cargo test -p sunlight-virtio --lib --offline --locked --target x86_64-unknown-linux-gnu' if reproducer and not is_native else None))
    write('host-fault-matrix.json', dict(status='PARTIAL', scope='hosted fixtures plus separately labeled native witnesses', cases=matrix))
    statuses=['PASS','PASS','PASS','PASS','PASS','PARTIAL','UNQUALIFIED','NOT_RUN','NOT_RUN','NOT_RUN','NOT_RUN','NOT_RUN','PARTIAL','NOT_RUN','NOT_RUN']
    titles=['Normal I/O baseline','Instrumented active request','Active request timeout','Reset with outstanding request','Independent reset completion','DMA cessation observation','Safe retirement','Failed/withheld reset','Late completion isolation','New epoch','Fresh post-reset I/O','Repeated recovery','Uncertain write provenance','Uncertain flush provenance','Malformed completion']
    write('native-fault-matrix.json', dict(status='PARTIAL', result=str(native_path.relative_to(OUT)), boots=native['boots'],
          cases=[dict(case=f'N{i}',title=title,status=status,scope='QEMU native guest; consult witnesses and explicit limits') for i,(title,status) in enumerate(zip(titles,statuses),1)],
          controls='six unmodified QEMU normal and six instrumented pre-reset release cases; neither is new-epoch recovery'))
    smp_cases=[]
    for i,title in enumerate(['Admission vs quiescence','Completion vs reset','Timeout vs late completion','Competing reset owners','Ack vs retirement','Reinit vs stale completion','IRQ vs teardown','Exhaustion vs admission'],1):
        smp_cases.append(dict(case=f'S{i}',title=title,status='NOT_RUN',reason='required competing native interleaving not forced'))
    write('smp-progress.json', dict(status='PARTIAL', configured_profiles=[1,2,4],
          actual_ap_startup='PASS: AP online messages precede test submission',
          native_timer_progress=[dict(cpu=r['cpu'],cache=r['cache'],progress=r['progress'],all_cpus_advance=r['all_configured_cpu_timer_progress']) for r in holds],
          scheduler_progress='NOT_RUN: SCHEDULER_READY=false in fixture', competing_context_reset='NOT_RUN',
          io_context='BSP with interrupts enabled after AP startup; local exclusive mutable driver owner',
          test_hook='per-CPU atomic increment before timer handler scheduler readiness check; no waits/allocations/locks',
          blocker='synchronous transfer retains exclusive mutable device ownership through polling; safe reset from another context requires a separately designed coordinator/staged API and post-scheduler fixture', cases=smp_cases))
    write('durability-uncertainty.json', dict(status='PARTIAL', native_uncertain_write='PASS: original Deadline receipt, Unknown amount, Uncertain durability survives reset/refusal',
          held_write_backing_image='all bytes unchanged because backend never submitted; does not generalize to other timeout points',
          release_control='actual payload reaches backing file and normal flush completes; no physical persistence claim',
          failed_uncertain_flush='hosted fixtures only in this milestone', reset_ack_implies_durable=False,
          reset_ack_implies_failed_without_side_effects=False, queue_incarnation=1, media_identity='boot-local attachment only',
          native_sunfs_mount=False, typed_downstream_gate='still incomplete', authoritative_short_transfer='still unqualified',
          future_recovery='must validate complete checkpoint closures and select valid durable generation'))
    blockers=[
      'No native memory-copy instruction or asynchronous DMA worker active concurrently with reset (H4).',
      'Post-fence original-mapping protection is demonstrated, but aliases/remapping and asynchronous DMA worker coverage remain unqualified.',
      'No qualified guest DMA retirement, frame ownership release or allocator reuse; all five pages stay pinned.',
      'No queue reinitialization/new epoch or post-reset fresh I/O.',
      'No native delayed/withheld reset, late completion, unplug, malformed completion or required racing schedules.',
      'AP timer progress is established; scheduler-ready progress and S1-S8 contention remain NOT_RUN.',
      'No global quarantine capacity/attachment-retry policy or native allocator-pressure test.',
      'Adversarial indistinguishable forged completions remain unauthenticatable on this wire protocol.',
      'Typed USB/cache/partition/FAT/VFS propagation, authoritative short transfers and physical durability remain separate gates.'
    ]
    write('remaining-blockers.json', dict(status='PARTIAL', blockers=blockers, recommended='SFS-5B.4: asynchronous DMA-access guard and post-scheduler reset coordinator qualification before retirement/new-epoch work', native_writable_sunfs='BLOCKED'))
    counts={}
    for row in hosted:
        log=(OUT/row['log']).read_text(errors='replace')
        counts[Path(row['log']).name]=sum(map(int,re.findall(r'test result: ok\. (\d+) passed',log)))
    corpora={}
    schedules={}
    corpus_hashes={}
    for profile in ['debug','release']:
        directory=hosted_path.parent/profile
        corpora[profile]={}
        schedules[profile]={}
        corpus_hashes[profile]={}
        for sub in ['sfs2','sfs3','sfs4']:
            files=[p for p in (directory/sub).rglob('*') if p.is_file()]
            corpora[profile][sub]=dict(files=len(files),json_files=len([p for p in files if p.suffix=='.json']))
            rows=[]
            for path in sorted((directory/sub).glob('crash-*.jsonl'))+list((directory/sub).glob('reordering.jsonl')):
                rows.extend(json.loads(line) for line in path.read_text().splitlines())
                corpus_hashes[profile][str(path.relative_to(directory))]=hashlib.sha256(path.read_bytes()).hexdigest()
            schedules[profile][sub]=dict(cases=len(rows),all_pass=all(row.get('status')=='PASS' for row in rows),
                                         selected_expected=all(row.get('selected') in row.get('expected',[]) for row in rows))
        assert sum(v['cases'] for v in schedules[profile].values()) == 13142
        assert all(v['all_pass'] and v['selected_expected'] for v in schedules[profile].values())
    assert corpus_hashes['debug'] == corpus_hashes['release']

    write('regression-results.json', dict(status='PARTIAL', available_commands_pass='PASS', input_integrity=inputs,
          hosted_commands=hosted, checks=checks, fresh_test_counts=counts, crash_corpora_inventory=corpora, crash_schedules=schedules, crash_corpus_hashes=corpus_hashes, debug_release_corpora_equal=True,
          native_boots=18, held_request_boots=6, release_controls=6, unmodified_controls=6,
          production_driver_changed=False, mandatory_missing='see native-fault-matrix.json and smp-progress.json',
          rust_format_scope='driver, contract, SFS5A/5B1/5B2/5B3 fixture files; raw/scoped-format.log',
          python_syntax='ast.parse of new runner and verifier', qemu_build='raw/qemu-build.log',
          limitations='hosted schedules are not native crashes; no physical-hardware qualification'))
    write('production-changes.json', dict(status='PASS', production_driver_changes=[],
          feature='sfs5b3-test off by default, inherits disposable sfs5a-test hook',
          changed_existing_files=['kernel/Cargo.toml','kernel/src/main.rs','kernel/src/arch/x86_64/interrupts.rs'],
          new_files=['kernel/src/sfs5b3_test.rs','tools/sunfs5b3_validate.py'],
          runtime_effect_when_disabled='none from added feature-gated hooks',
          commit=False,push=False,native_sunfs_mount=False))
    write('validation-summary.json', dict(milestone='SFS-5B.3',status='PARTIAL',complete=False,
          scope='native SunlightOS in isolated QEMU11.1.2 legacy PCI TCG; partial real write-payload consumption before backend',
          gates={'G1':'PASS','G2':'PASS','G3':'PARTIAL','G4':'PARTIAL','G5':'PARTIAL','G6':'UNQUALIFIED','G7':'PASS','G8':'PARTIAL','G9':'PARTIAL','G10':'PASS'},
          gate_limits={'G3':'independent real DMA reads witnessed; active DMA during reset absent; original mapped-range guard only',
                       'G4':'real outstanding request reset; required native fault races and H4 missing',
                       'G5':'normative conforming-device boundary plus narrow QEMU cancellation; general reuse not qualified',
                       'G6':'zero guest DMA resources reclaimed', 'G7':'bounded terminal fail-stop only, not recovery/reinitialization',
                       'G8':'post-AP timer progress, no scheduler-ready contention', 'G9':'available regressions pass; mandatory full native matrix incomplete'},
          active_dma_reset='UNQUALIFIED',dma_retirement='UNQUALIFIED',physical_fencing='UNQUALIFIED',
          native_writable_sunfs='BLOCKED',commit=False,push=False))
    print('derived PARTIAL ledger from',native_path.name, hosted_path.name)


if __name__ == '__main__':
    main()
