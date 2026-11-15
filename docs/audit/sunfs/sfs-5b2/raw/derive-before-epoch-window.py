#!/usr/bin/env python3
"""Derive scoped verdicts from retained executions; never turn missing tests into PASS."""
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(__file__).resolve().parent
RAW = OUT/'raw'
REPORT = ROOT/'docs/filesystems/sunfs/sfs-5b2-virtio-fencing-dma-retirement.md'
REQUIRED = ['source-baseline.json', 'virtio-specification-matrix.md', 'transport-feature-inventory.json',
 'descriptor-aba-witness.json', 'normal-completion-lifecycle.json', 'fencing-contract.md',
 'reset-state-machine.json', 'dma-retirement.json', 'quarantine-reconciliation.json',
 'fault-injection-matrix.json', 'independent-witnesses.json', 'native-reset-evidence.json',
 'smp-progress.json', 'durability-uncertainty.json', 'remaining-blockers.json',
 'regression-results.json', 'production-changes.json', 'source-integrity.json', 'validation-summary.json']


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def load(p):
    return json.loads(p.read_text())


def write(name, value):
    (OUT/name).write_text(json.dumps(value, indent=2)+'\n')


def rel(p):
    return str(p.relative_to(OUT))


def runner():
    s = importlib.util.spec_from_file_location('runner', ROOT/'tools/sunfs5b2_validate.py')
    m = importlib.util.module_from_spec(s)
    s.loader.exec_module(m)
    return m


def final_runs():
    current = runner().base.source()
    found = {}
    for mode in ['native', 'hosted', 'regression-native', 'checks']:
        candidates = [p for p in sorted(RAW.glob(mode+'-*')) if p.is_dir()
                      and (p/'input-integrity.json').exists()
                      and load(p/'input-integrity.json')['status'] == 'PASS'
                      and load(p/'source-before.json') == current
                      and load(p/'source-after.json') == current]
        if candidates:
            found[mode] = candidates[-1]
    return current, found


def derive():
    current, runs = final_runs()
    native = runs.get('native')
    hosted = runs.get('hosted')
    regression = runs.get('regression-native')
    checks = runs.get('checks')
    n = load(native/'native-result.json') if native else {'status':'NOT_RUN','boots':[]}
    h = load(hosted/'hosted-result.json') if hosted else []
    rn = load(regression/'native-result.json') if regression else {'status':'NOT_RUN','boots':[]}
    c = load(checks/'checks-result.json') if checks else []
    common = {'milestone':'SFS-5B.2', 'scope':'coherent x86_64 legacy PCI split ring in QEMU; no physical device qualification'}
    sources = ['sunlight-virtio/src/blk.rs','sunlight-virtio/src/lib.rs','kernel/src/main.rs',
               'kernel/Cargo.toml','kernel/src/sfs5b2_test.rs','tools/sunfs5b2_validate.py']
    write('production-changes.json', dict(common, status='PASS', changed_from_incoming={p:sha(ROOT/p) for p in sources},
      changes=['Bound initial reset status-zero acknowledgment before queue publication',
       'Reject non-running device status before request DMA publication and before logical retirement',
       'Exclusive one-attempt terminal reset with at most 64 one-read poll steps; no wait/allocate/retry',
       'Retain original uncertain receipt independently of reset and subsequent refusals',
       'Expose offered/negotiated feature and diagnostic reset observations; no retirement authority',
       'Add hosted transition tests and off-by-default disposable native reset fixture/runner'],
      normal_descriptor_algorithm='Existing exact owned-chain/index/status/token validation retained; no claim of repaired malicious same-head alias',
      untouched_incoming=['Cargo.lock','sunlight-block/src/contract.rs','sunlight-block/src/lib.rs','sunlight-block/src/usb_msc.rs',
         'sunlight-virtio/Cargo.toml','kernel/src/sfs5a_test.rs','kernel/src/sfs5b1_test.rs','tools/sunfs5a_validate.py','tools/sunfs5b1_validate.py'],
      commit=False,push=False,native_sunfs_mount=False))
    feature_rows=[]
    witnesses=[]
    for boot in n['boots']:
        serial = (native/boot['serial']).read_text()
        offered, accepted = map(int,re.search(r'features=\((\d+), (\d+)\)',serial).groups())
        feature_rows.append({'cpu':boot['cpu'],'cache':boot['cache'],'offered':offered,'offered_hex':hex(offered),
          'accepted':accepted,'accepted_hex':hex(accepted),'accepted_bits':[i for i in range(32) if accepted & (1<<i)],
          'unaccepted_offered_bits':[i for i in range(32) if offered & ~accepted & (1<<i)],'serial':rel(native/boot['serial'])})
        trace=(native/boot['trace']).read_text()
        devices=set(re.findall(r'virtio_blk_handle_(?:read|write) vdev (0x[0-9a-f]+)',trace))
        assert len(devices)==1
        vdev=next(iter(devices))
        completed=re.findall(r'virtio_blk_req_complete vdev '+vdev+r' req (0x[0-9a-f]+) status (\d+)',trace)
        assert len(completed)==4 and all(status=='0' for _,status in completed), completed
        resets=re.findall(r'virtio_set_status vdev '+vdev+r' val (\d+)',trace)
        qmp=load(native/boot['qmp'])
        witnesses.append(dict(cpu=boot['cpu'],cache=boot['cache'],vdev=vdev,request_completions=completed,
           status_write_trace=resets,driver_ack_zero='state: VerifiedRetained, reads: 1, status: Some(0)' in serial,
           trace=rel(native/boot['trace']),qmp=rel(native/boot['qmp']),observer_error=qmp.get('error'),
           independent_scope='QEMU request handling/completion and reset ENTRY, standard QMP PCI/block inventory; not independent DMA drain or reset completion',
           no_active_dma=True))
    write('transport-feature-inventory.json',dict(common,status='PASS' if feature_rows else 'NOT_RUN',
      transport='PCI legacy BAR0 port I/O; vendor 0x1af4/device 0x1001 in declared native fixture',queue=0,
      layout='split, 4096-byte legacy used-ring alignment', packed_supported=False, modern_block_supported=False,
      legacy_feature_width=32,allowlist_bits=[1,5,6,9],features=feature_rows,msix='not enabled by this driver; legacy config offset 0x14',
      ring_reset={'bit':40,'negotiated':False,'available':False},version_1={'bit':32,'negotiated':False},packed={'bit':34,'negotiated':False},
      pci_bdf='00:02.0',io_base='0xc000',queue_size='power-of-two in 4..256 by allocation guard; numeric native value not logged before reset',
      identity='boot-local attachment incarnation 1; configured serial is harness identity, not persistent guest-read media identity',
      iommu='no IOMMU isolation or translation teardown qualification',source_audit=['sunlight-virtio/src/{blk,pci}.rs','kernel/src/main.rs:1861..2190',
          'kernel boot block initialization precedes smp::start_aps; no block IRQ completion handler found']))
    write('descriptor-aba-witness.json',dict(common,status='PARTIAL',failure_preserved=True,repair_complete=False,
      failure_log='raw/aba-pre-repair-assertion.log',failing_source='raw/aba-pre-repair-assertion.rs',
      initial_preproduction_marker_failure='raw/descriptor-aba-failing.log',initial_marker_source='raw/aba-failing-source.rs',
      repro_command='raw/aba-reproducer-command.json',incoming_source='raw/incoming/sunlight-virtio/src/blk.rs',
      first_attempt='raw/baseline-witness-first.log used --exact without full module path and selected ZERO tests; not evidence',
      rerun='raw/baseline-witness.log executed original demonstrated-limit regression',
      device={'backend':'VirtioLegacy','incarnation':1},queue={'index':0,'incarnation':1},head=0,
      A={'generation':1,'used_before':0,'used_after':1,'avail_after':1,'transitions':['Prepared','Submitted','Completed','Retired']},
      B={'generation':2,'used_before':1,'used_after':2,'avail_after':2,'transitions':['Prepared','Submitted','Completed','Retired']},
      injection={'delta':1,'head':0,'len':1,'status':0,'corrupt':False,'timing':'after B publication; fake ring AND B status slot written'},
      validation=['next used index','head matches current token','status 0','owned descriptors match','current token completed/retired'],
      invalid_acceptance='forged old head plus B status accepted as B Completed; device transmitted no generation',
      classification='adversarial duplicate after legitimate successful retirement/reuse; NOT timeout reuse, duplicate consumption, index wrap or demonstrated compliant-device behavior',
      head_only_case='f4_head_only_duplicate_cannot_supply_new_status: B status remains sentinel; queue quarantines',
      unresolved='Physically identical B completion and malicious old-head-plus-B-status cannot be disambiguated; current conforming-device reuse assumption does not meet arbitrary adversarial wire guarantee'))
    write('normal-completion-lifecycle.json',dict(common,status='PASS',scope='conforming device only; logical request retirement, no PMM release',
      lifecycle=['Created/Retired','Prepared','Submitted','Completed','Retired'],
      validation=['live device status exactly ACKNOWLEDGE|DRIVER|DRIVER_OK before publication and retirement',
       'used_idx == last_used_idx.wrapping_add(1)','used head equals current head 0',
       'all chain addresses/lengths/flags/next pointers match exact header/data/status layout',
       'status initialized 0xff before publication and must be 0/1/2 on completion',
       'Owner active token matches device, queue incarnation, generation and Submitted state',
       'used index consumed once before exact-token retirement',
       'owner.complete/retire reject repeated and stale tokens'],
      ordering='SeqCst fences around available publication and used consumption on coherent x86; not a storage barrier',
      wrap='ring_index_wrap_is_separate_from_generation_exhaustion; protocol u16 wraps, request u64 generation refuses exhaustion',
      interrupts='polling only; interrupt notification carries no completion authority',
      evidence=['storage regression log','kernel ownership contract tests','native serial and QEMU completion traces'],
      limitations=['same-head malicious status writer remains unresolved','no queue epoch reinitialization implemented','legacy used.len not authoritative']))
    write('reset-state-machine.json',dict(common,status='PARTIAL',
      transitions={'Running':'begin_terminal_reset closes admission/quarantines before one whole-device status-zero write',
       'Waiting':'poll_terminal_reset performs one status read, at most 64 calls',
       'VerifiedRetained':'status zero acknowledgment; no reopen/release/reinitialization',
       'TimedOutRetained':'64 reads without zero; no DMA-stop claim',
       'UnexpectedStatusRetained':'status other than 0 or 7, including 0xff; no DMA-stop claim'},
      coordinator='exclusive &mut VirtioBlk, existing external mutex for shared access; no new coordinator lock',
      competing_begin='same diagnostic state, no second write, no budget restart, no owner replacement',
      terminal='all non-Running states reject I/O; repeated poll/begin immutable after terminal outcome',
      not_implemented=['OldEpochRetired','Reinitializing','RunningNewEpoch'],
      normative='VirtIO 1.4 2.4.1/2.4.2 with legacy PCI status layout 4.1.4.10; 3.3/3.3.1 closes live queues on reset',
      work_bound='initialization <=64 status reads; runtime begin one write; runtime poll one read, <=64 total; port-I/O completion assumes responsive platform',
      not_proven=['wall-clock bound on hardware register instruction','active-SMP coordination','platform DMA drain']))
    resource_types=['4 contiguous queue frames: descriptor/available/used rings','1 bounce/request frame: header+data+status',
                    'logical chain 0/1(/2)','active RequestId and owner state','indices and last outcome','first uncertain receipt']
    write('dma-retirement.json',dict(common,status='UNQUALIFIED',resources=resource_types,pages_per_attachment=5,
      bytes_per_attachment=20480,hosted_fixture='two aligned 16KiB arrays (queue plus oversized request fixture); fake DMA only, not five native PMM frames',pages_released=0,bytes_released=0,remaining_pinned_pages=5,
      normal='logical request retirement only after valid completion; five frames remain allocated for lifetime',
      reset='acknowledgment never invokes frame free, descriptor reuse, owner retirement, DMA unmap or new epoch',
      actual_native_dma_at_reset=False,independent_dma_drain_witness=False,
      limitation='no independent active-DMA/translation/posted-write retirement proof; no frame-free authority exposed'))
    write('quarantine-reconciliation.json',dict(common,status='PARTIAL',before_timeout='one Submitted token; 5 pinned pages',
      after_timeout='same active token, Quarantined; first uncertain receipt retained',after_ack='same active token and Quarantined; VerifiedRetained; 5 pinned pages',
      fenced_release_count=0,remaining_uncertain_requests_hosted=1,remaining_uncertain_requests_native_idle_reset=0,
      outstanding_native_reset='None (all four preceding requests completed)',
      pressure='1024 further submissions/reset initiations refused/idempotent without changing ring or request bytes; no growth, no quarantine eviction',
      accounting_scope='per attachment, not sum across separate VMs; pages from PMM allocation source, not an independent PMM instrumentation witness'))
    cases=[
     ('F1','Normal delayed completion','NOT_RUN',None,'normal native completions observed; deliberate I/O delay absent'),
     ('F2','Timeout before completion','PASS','timeout_must_not_reuse_dma_page_or_descriptors','fake ring never advances'),
     ('F3','Completion after software timeout','PASS','late_completion_must_not_authorize_new_request','fake ring/status write after timeout; caller untouched'),
     ('F4','Duplicate completion','PARTIAL','f4_head_only_duplicate_cannot_supply_new_status','head-only duplicate quarantines; forged head+new status still aliases'),
     ('F5','Invalid descriptor head','PASS','t15_invalid_used_head_and_index','fake head99 or delta2; active owner retained'),
     ('F6','Valid head, wrong chain','PASS','t16_descriptor_ownership_and_bad_status','fake descriptor address mismatch; no retirement'),
     ('F7','Used index wrap','PASS','ring_index_wrap_is_separate_from_generation_exhaustion','65535 to0, generation1'),
     ('F8','Stale interrupt','PARTIAL','f3_f8_late_ring_write_and_stale_notification_do_not_reopen_reset','stale ring modeled; actual IRQ injection absent; polling path'),
     ('F9','Reset with active DMA','PARTIAL','f9_f21_f22_reset_retains_uncertain_io_and_all_dma_bytes','fake uncertain owner reset; native reset IDLE only'),
     ('F10','Reset completion delayed','PASS','f10_delayed_ack_has_exact_read_bound','63 status7 reads then0 at64; fake registers'),
     ('F11','Reset acknowledgment withheld','PASS','f11_withheld_ack_is_terminal_and_not_a_fence','64 fake status7 reads; later0 ignored'),
     ('F12','Reset unsupported','PARTIAL',None,'queue reset unavailable by transport/features; runtime unsupported whole-device reset scenario absent'),
     ('F13','Unexpected device state','PASS','f13_device_lost_running_state_blocks_dma_publication','0/3/0x47/0x87/0xff admission refuses; fake reset unexpected status tests also pass'),
     ('F14','Old completion in new epoch','NOT_RUN',None,'new epoch unavailable; no old mappings transferred'),
     ('F15','Reuse before fence','PASS','f15_f17_f18_reset_competitors_and_pressure_preserve_first_owner','uncertain owner prevents all further submission'),
     ('F16','Removal during reset','PARTIAL','f13_f16_unexpected_or_removed_device_cannot_ack','0xff register model only; no actual unplug'),
     ('F17','Quarantine pressure','PASS','f15_f17_f18_reset_competitors_and_pressure_preserve_first_owner','one timeout then1024 refusals; not1024 active uncertain requests'),
     ('F18','Concurrent reset initiators','PARTIAL','f15_f17_f18_reset_competitors_and_pressure_preserve_first_owner','serialized competing begin calls only, no concurrent CPU race'),
     ('F19','Reset versus normal completion','PARTIAL','f19_normal_retirement_then_reset_has_no_second_retirement','adjacent serialized events, not actual racing interleaving'),
     ('F20','Reinitialize after fence','NOT_RUN',None,'no safe retirement/reinit API implemented'),
     ('F21','Uncertain write then reset','PASS','f9_f21_f22_reset_retains_uncertain_io_and_all_dma_bytes','fake write timeout then status0; unchanged uncertainty and bytes'),
     ('F22','Uncertain flush then reset','PASS','f9_f21_f22_reset_retains_uncertain_io_and_all_dma_bytes','fake flush timeout then status0; unchanged uncertainty and bytes')]
    rows=[]
    for case,title,status,test,description in cases:
        rows.append(dict(id=case,scenario=title,status=status,mechanism=description,test=test,
          evidence_scope='hosted fake ring/register model unless explicitly described otherwise',
          actual_device_operation=False,actual_dma_in_flight=False,actual_transport_reset=False,
          normative_guarantee='model tests establish software retention only; 2.4 reset requirements require actual device acknowledgment',
          before_after='named assertions prove the stated state/byte transitions; no additional per-case raw ring dump captured',wire_echoes_generation=False,wire_echoes_queue_incarnation=False,software_request_identity={'device_incarnation':1,'queue_incarnation':1,'head':0,'generation':2 if case=='F4' else None if case in ['F1','F10','F11','F12','F13','F14','F16','F20'] else 1},
          retirement='zero frame release; quarantined tokens not retired',progress='finite fixture/test iteration bounds',
          final_queue='Quarantined/retained for fault cases; Retired for conforming completion/wrap',seed='none: deterministic tuples'))
    if hosted:
        storage_text=(hosted/'storage.log').read_text()
        for row in rows:
            if row['test']:
                assert 'test blk::ownership_tests::'+row['test']+' ... ok' in storage_text, row['test']
    write('fault-injection-matrix.json',dict(common,status='PARTIAL',tests=rows,
      native_faults='post-reset I/O refusal/idempotent reset exercised natively; no native active-DMA injection',
      native_normal_and_reset=rel(native/'native-result.json') if native else None,
      missing='native delayed completion, withheld reset, unplug, old/new epoch interleavings; no fabricated PASS'))
    write('independent-witnesses.json',dict(common,status='PARTIAL',
      A='Driver Owner state, receipts, queue incarnation, reset status readback and byte-retention assertions',
      B='QEMU trace backend observes actual block request handling/completion and device reset entry independently of driver counters; standard QMP inventory observes attached disposable device',
      observations=witnesses,agreement_scope='four actual requests completed before idle reset; device reset was requested and driver read zero',
      no_independent_ack_or_dma_drain=True,
      observer_failure='first unrestricted attempt used x-query-virtio-status after reset; QEMU 11.1.2 asserted in visit_start_struct; preserved .qemu/.qmp logs. Original runner PASS rows describe guest+image only and are superseded for observer qualification',
      emulator_modifications=False,physical_qualification=False,
      unavailable_source='matching QEMU tag source fetch failed: GitLab HTTP403, GitHub connection failure including unrestricted retry; no implementation-level reset-drain claim'))
    native_cases={'N1':'PASS','N2':'PASS','N3':'PASS','N4':'PASS','N5':'PASS','N6':'NOT_RUN','N7':'NOT_RUN','N8':'NOT_RUN',
                  'N9':'NOT_RUN','N10':'PARTIAL','N11':'UNQUALIFIED','N12':'PARTIAL','N13':'NOT_RUN','N14':'NOT_RUN','N15':'PARTIAL'}
    write('native-reset-evidence.json',dict(common,status='PARTIAL',attempts=[rel(p) for p in sorted(RAW.glob('native-*')) if p.is_dir()],
      final_attempt=rel(native) if native else None,boots=n['boots'],cases=native_cases,
      case_scope={'N5':'actual idle status-zero write plus observed zero readback; no active DMA',
       'N10':'all post-reset I/O refused; no new epoch','N12':'queue-reset absence; whole-device unsupported branch not executed',
       'N15':'post-reset repeated begin and refusal only; no interrupted active coordinator'},
      exact_image='only sector61 becomes 0xb2; marker and all remaining bytes unchanged; compressed whole images retained',
      device='sole disposable virtio-blk regular file, guest booted from ISO; no root/boot data disk attached',
      source_integrity=[rel(p/'input-integrity.json') for p in runs.values()],
      preservation='sandbox socket-denied attempt and QEMU observer assertion attempts retained, not promoted to full PASS'))
    progress=[
      ('P1','PARTIAL','native idle reset acknowledgment at read1; hosted acknowledgment at read64'),
      ('P2','PARTIAL','hosted64 reads returns TimedOutRetained; native withheld acknowledgment absent'),
      ('P3','NOT_RUN','serialized adjacent completion/reset test only'),
      ('P4','PARTIAL','1024 repeated begin calls immutable; active-CPU contention absent'),
      ('P5','PARTIAL','hosted late ring write before ack retains active token; native race absent'),
      ('P6','PARTIAL','I/O before/after quiescence serialized via exclusive mutable owner; actual admission race absent'),
      ('P7','PARTIAL','1024 post-timeout refusals with unchanged bytes; native timeout absent'),
      ('P8','NOT_RUN','no active-SMP secondary-CPU operations')]
    write('smp-progress.json',dict(common,status='PARTIAL',configured_cpus=[1,2,4],cache_modes=['writeback','none'],
      ap_startup=False,secondary_execution=False,post_ap_io=False,reset_contention=False,independent_cpu_progress='NOT_RUN',
      execution='BSP before start_aps, all native fixtures halt before AP startup',
      progress=[dict(id=i,status=s,witness=w) for i,s,w in progress],
      review={'new_locks':0,'new_allocations_during_reset':0,'reset_interrupt_dependency':False,'reset_timer_dependency':False,
       'runtime_poll':'one port read per call,64 maximum; no lock acquisition in driver',
       'existing_submit':'50,000,000 iteration polling under external device mutex; VFS outer-lock contexts remain a latency/ordering blocker',
       'memory_ordering':'coherent x86 SeqCst barriers, no noncoherent/IOMMU claim',
       'identity':'no queue epoch change/reinit; u64 request generation remains checked',
       'instruction_progress':'register instruction/emulator trap response itself not a software-time bound',
       'unqualified':['native contention/starvation','reset versus in-progress software completion','physical DMA mappings/posted writes','device reincarnation/hot removal']}))
    write('durability-uncertainty.json',dict(common,status='PASS',scope='hosted receipt semantics; no native uncertain reset injection',
      uncertain_write='Uncertain/Unknown/Uncertain remains identical in first_uncertain after acknowledged model reset',
      uncertain_flush='Uncertain/Unknown/Uncertain remains identical in first_uncertain after acknowledged model reset',
      reset_receipt='separate diagnostic type cannot construct an IoOutcome success or retirement permission',
      completed_write='NotEstablished durability, never durable merely on completion/reset',
      native_flush='real negotiated flush returns VirtualBarrier only for completed prior generation in that queue',
      writable_native_sunfs='BLOCKED; future adapter needs fenced recovery plus durable checkpoint closure validation'))
    blockers=['malicious old-head plus new-status alias after legitimate reuse remains reproducible; generation is not transmitted',
      'no independent active-DMA reset/quiescence witness or physical/platform translation retirement proof',
      'no old-epoch retirement, DMA frame release, safe queue reinitialization or post-fence I/O',
      'no native timeout/delayed-DMA/withheld-reset/removal/interleaving fault injection',
      'no active AP/SMP I/O-reset contention or scheduler progress qualification',
      'typed USB/cache/partition/FAT/VFS receipt propagation remains incomplete',
      'legacy short-transfer byte counts not independently authoritative; T5 remains PARTIAL',
      'USB MSC qualification and actual media-removal uncertainty remain open; T20 remains PARTIAL',
      'physical storage durability and hostile-device containment unqualified']
    write('remaining-blockers.json',dict(common,status='PARTIAL',blockers=blockers,
      next_milestone='SFS-5B.3: first build an independently observable active-I/O reset backend and a justified epoch/retirement protocol, then qualify post-AP contention; keep quarantine and prohibit writable SunFS',
      unrelated_changes='USB, filesystem formats, CPython, SunApp, scheduler and root migration unchanged'))
    crash={};corpora={}
    if hosted:
        for profile in ['debug','release']:
            crash[profile]={};corpora[profile]={}
            for sub in ['sfs2','sfs3','sfs4']:
                rows=[]
                for p in sorted((hosted/profile/sub).glob('crash-*.jsonl'))+list((hosted/profile/sub).glob('reordering.jsonl')):
                    rows.extend(json.loads(line) for line in p.read_text().splitlines())
                    corpora[profile][str(p.relative_to(hosted/profile))]=sha(p)
                crash[profile][sub]={'cases':len(rows),'all_pass':all(x.get('status')=='PASS' for x in rows),
                  'selected_expected':all(x.get('selected') in x.get('expected',[]) for x in rows),
                  'seeds':sorted(set(x['seed'] for x in rows if 'seed' in x))}
    if hosted:
        old=load(ROOT/'docs/audit/sunfs/sfs-5b1/negative-tests.json')
        fresh=[]
        text=(hosted/'storage.log').read_text()
        for row in old['tests']:
            assert row['test']+' ... ok' in text,row['test']
            fresh.append({**{k:row[k] for k in ['id','test','status','injection','expected','observed','ownership','retirement','execution']},
                'log':rel(hosted/'storage.log'),'source_manifest':rel(hosted/'source-before.json'),
                'actual_dma':False,'seed':'none; deterministic tuples','native_supplement':'prior native evidence not promoted to final-source execution'})
        write('raw/t1-t20-final-matrix.json',dict(status='PARTIAL',tests=fresh,metadata_origin='prior matrix classifications retained; all named hosted regressions freshly verified'))
    counts={}
    if hosted:
        for p in hosted.glob('*.log'):
            count=sum(map(int,re.findall(r'test result: ok\. (\d+) passed',p.read_text())))
            if count:counts[p.name]=count
    regression_ok=bool(h) and bool(c) and all(x['returncode']==0 for x in h+c) and rn['status']=='PASS' and n['status']=='PASS'
    if regression_ok:
        regression_ok=bool(corpora['debug']) and corpora['debug']==corpora['release'] and all(
           x['cases']>0 and x['all_pass'] and x['selected_expected'] for p in crash.values() for x in p.values())
    write('regression-results.json',dict(common,status='PASS' if regression_ok else 'NOT_RUN',
      runs={m:rel(p) for m,p in runs.items()},commands=h,checks=c,test_counts=counts,crash_schedules=crash,
      crash_corpus_hashes=corpora,debug_release_corpora_equal=bool(corpora) and corpora.get('debug')==corpora.get('release'),
      expected_schedules_per_profile=13142,native_regression=rn,native_reset=n['status'],
      matrix_T1_T20='fresh block/VirtIO suites rerun original named tests; T5/T17/T20 qualification stays PARTIAL, even when their bounded regression assertions pass',
      preserved_failures=['raw/descriptor-aba-failing.log','raw/baseline-witness-first.log','sandbox socket-denied native attempt',
       'QEMU experimental-query abort attempt','pre-final runner input-integrity mismatch when corrected observer runner changed'],
      missing=['native active-DMA faults','active-SMP progress','reinitialization and actual post-fence release'],historical_passes_reused=False))
    gates={'G1':'PASS' if feature_rows else 'PARTIAL','G2':'PARTIAL','G3':'PASS','G4':'PARTIAL','G5':'PARTIAL','G6':'UNQUALIFIED',
           'G7':'PARTIAL','G8':'PARTIAL','G9':'PARTIAL','G10':'NOT_RUN'}
    write('validation-summary.json',dict(common,status='PARTIAL',gates=gates,
      available_final_source_regressions='PASS' if regression_ok else 'NOT_RUN',
      gate_limits={'G2':'original malicious wire ambiguity not repaired','G3':'normal conforming exactly-once logical retirement only',
       'G4':'model safety passes; required native negative interleavings absent',
       'G5':'native idle reset acknowledgment only; active outstanding-DMA fencing not proven',
       'G6':'no DMA frame release qualified; all pages retained',
       'G7':'independent request/reset-entry witness; native fault interleavings absent',
       'G8':'hosted bounded reset; no active SMP',
       'G9':'available full regressions pass when recorded; mandatory native fault/recovery/SMP cases not all executed'},
      writable_native_sunfs='BLOCKED',physical_device_qualification='UNQUALIFIED',blockers=blockers,
      commit=False,push=False))
    print('derived',list(runs), 'regressions',regression_ok)


def seal(check=False):
    current, runs=final_runs()
    baseline=load(OUT/'source-baseline.json')
    previous={n:v for n,v in baseline['files'].items() if n.startswith(('docs/audit/sunfs/','docs/filesystems/sunfs/'))}
    bad=[n for n,v in previous.items() if not (ROOT/n).is_file() or sha(ROOT/n)!=v]
    assert not bad, bad
    if check:
        manifest=load(OUT/'source-integrity.json')
        bad=[n for n,v in manifest['sealed'].items() if not (ROOT/n).is_file() or sha(ROOT/n)!=v]
        assert not bad,bad
        assert current==manifest['execution_inputs']
        outputs={str(p.relative_to(ROOT)) for p in OUT.rglob('*') if p.is_file() and '__pycache__' not in p.parts and p.name!='source-integrity.json'}
        assert outputs <= set(manifest['sealed']), outputs-set(manifest['sealed'])
        print('PASS: final inputs, evidence seal and prior SFS evidence unchanged')
        return
    assert set(runs)=={'native','hosted','regression-native','checks'}, list(runs)
    assert all((OUT/n).is_file() for n in REQUIRED if n!='source-integrity.json')
    assert REPORT.is_file()
    assert len(load(OUT/'fault-injection-matrix.json')['tests'])==22
    assert load(OUT/'regression-results.json')['status']=='PASS'
    for profile,row in load(OUT/'regression-results.json')['crash_schedules'].items():
        assert sum(x['cases'] for x in row.values())==13142, (profile,row)
    allowed={'sunlight-virtio/src/blk.rs','sunlight-virtio/src/lib.rs','kernel/src/main.rs','kernel/Cargo.toml'}
    changed_old={n for n,v in baseline['files'].items() if not (ROOT/n).is_file() or sha(ROOT/n)!=v}
    assert changed_old <= allowed, changed_old-allowed
    status=load(OUT/'validation-summary.json');status['gates']['G10']='PASS';write('validation-summary.json',status)
    for name in ['sunlight-virtio/src/blk.rs','sunlight-virtio/src/lib.rs','kernel/src/main.rs','kernel/Cargo.toml',
                 'kernel/src/sfs5b2_test.rs','tools/sunfs5b2_validate.py']:
        p=RAW/'final-source'/name;p.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/name,p)
    patch=bytearray()
    for name in ['sunlight-virtio/src/blk.rs','sunlight-virtio/src/lib.rs','kernel/src/main.rs','kernel/Cargo.toml',
                 'kernel/src/sfs5b2_test.rs','tools/sunfs5b2_validate.py']:
        old=RAW/'incoming'/name
        result=subprocess.run(['git','diff','--no-index','--binary',str(old) if old.exists() else '/dev/null',str(ROOT/name)],cwd=ROOT,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        assert result.returncode in (0,1),result.stderr
        patch.extend(result.stdout)
    (RAW/'milestone.patch').write_bytes(patch)
    names=set(current)|{str(p.relative_to(ROOT)) for p in OUT.rglob('*') if p.is_file() and '__pycache__' not in p.parts}|{str(REPORT.relative_to(ROOT))}
    names.discard(str((OUT/'source-integrity.json').relative_to(ROOT)))
    sealed={n:sha(ROOT/n) for n in sorted(names)}
    write('source-integrity.json',{'status':'PASS','sealed':sealed,'execution_inputs':current,
      'final_runs':{m:rel(p) for m,p in runs.items()},'previous_sfs_files_verified':len(previous),
      'changed_incoming_files':sorted(changed_old),'scope':'all final execution inputs and milestone outputs, excluding this manifest and Python bytecode',
      'failed_and_superseded_attempts':'retained and sealed; not required to share final runner hash',
      'commit':False,'push':False})
    print('sealed',len(sealed),'files; preserved prior SFS files',len(previous))


if __name__=='__main__':
    if '--check' in sys.argv:seal(True)
    elif '--seal' in sys.argv:seal()
    else:derive()
