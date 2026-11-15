#!/usr/bin/env python3
"""Derive scoped evidence from immutable raw attempts; no baseline rewrites."""
import json
import hashlib
import re
from pathlib import Path

OUT = Path(__file__).resolve().parent
FINAL_INPUTS = ['kernel/Cargo.toml', 'kernel/build.rs', 'kernel/src/main.rs', 'kernel/src/arch/x86_64/interrupts.rs', 'kernel/src/arch/x86_64/syscall.rs', 'sunlight-virtio/src/blk.rs', 'kernel/src/sfs_reset.rs', 'kernel/src/sfs5b4_test.rs', 'tools/helios-probes/sfs5b4-probe.S', 'tools/sunfs5b4_validate.py', 'Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'rust-toolchain.toml']


def put(name, value):
    (OUT/name).write_text(json.dumps(value, indent=2)+'\n')


def load(p):
    return json.loads(p.read_text())


def main():
    candidates = sorted((OUT/'raw').glob('native-20*/native-result.json'))
    identity = load(OUT/'qemu-source-integrity.json')
    def device_inputs_match(result):
        for row in load(result).get('boots', []):
            inv = load(result.parent/row['serial'].replace('.serial.log', '.invocation.json'))
            if inv['patch_sha256'] != identity['patch_sha256']: return False
            if row['mode'] != 'unmodified' and inv['qemu_sha256'] != identity['binary_sha256']: return False
        return True
    def final_source_matches(result):
        frozen = load(result.parent/'source-before.json')
        return all(frozen.get(n)==hashlib.sha256((OUT.parents[3]/n).read_bytes()).hexdigest() for n in FINAL_INPUTS)
    final = next((p for p in reversed(candidates) if device_inputs_match(p) and len(load(p).get('boots', [])) == 42 and (p.parent/'input-integrity.json').is_file() and load(p.parent/'input-integrity.json')['status']=='PASS' and final_source_matches(p)), None)
    if not final:
        raise SystemExit('No complete 42-case attempt; raw attempts preserved')
    data = load(final)
    rows = data['boots']
    raw = final.parent
    rel = str(final.relative_to(OUT))
    held = [r for r in rows if r['mode'] in ['hold', 'reset-read', 'error', 'negative']]
    neg = [r for r in rows if r['mode']=='negative']
    safe = [r for r in held if r['mode']!='negative']
    good = all(r['status']=='PASS' for r in rows)
    graphs = []
    for r in rows:
        nodes, edges = [], []
        guests = sorted(r['guest'], key=lambda x:x['seq'])
        devices = r['device']
        for domain, events in [('g', guests), ('d', devices)]:
            nodes += [dict(id=f'{domain}{e["seq"]}', domain='NATIVE_OBSERVED' if domain=='g' else 'INSTRUMENTED_QEMU', **e) for e in events]
        # Program-order guest edges are per task incarnation only. Atomic log
        # counter order between CPUs does not itself prove happens-before.
        for actor in set(e['task_generation'] for e in guests):
            events = [e for e in guests if e['task_generation']==actor]
            edges += [dict(before=f'g{a["seq"]}', after=f'g{b["seq"]}', basis='same-task program order across native yields') for a,b in zip(events,events[1:])]
        edges += [dict(before=f'd{a["seq"]}',after=f'd{b["seq"]}',basis='BQL serialized device operation order') for a,b in zip(devices,devices[1:])]
        def edge(a, b, basis):
            aa = next((e['id'] for e in nodes if e['stage']==a and e['domain']==('NATIVE_OBSERVED' if a in ['SUBMISSION_INITIATED','DEVICE_ACCESS_HANDSHAKE','RESET_REQUESTED'] else 'INSTRUMENTED_QEMU')), None)
            bb = next((e['id'] for e in nodes if e['stage']==b and e['domain']==('NATIVE_OBSERVED' if b in ['DEVICE_ACCESS_HANDSHAKE','RESET_PENDING'] else 'INSTRUMENTED_QEMU')), None)
            if aa and bb: edges.append(dict(before=aa,after=bb,basis=basis))
        edge('SUBMISSION_INITIATED','DEVICE_FETCHED','guest log before descriptor publication/notify; device pops that sector-63 head')
        edge('GUEST_WITNESS_PUBLISHED','DEVICE_ACCESS_HANDSHAKE','device status byte 0xfe after real copy; guest volatile read observes it; used.idx unchanged')
        edge('RESET_REQUESTED','RESET_ENTERED','guest log before synchronous legacy status-zero outb; device reset handler')
        edge('ORIGINAL_MAPPINGS_PROTECTED','RESET_PENDING','BQL reset/guard returns before guest outb returns and pending log')
        for a, b, basis in [
            ('TIMEOUT_QUARANTINE', 'QUIESCING', 'timeout log precedes TIMEOUT release; reset contender acquires TIMEOUT before closing admission'),
            ('NORMAL_COMPLETION', 'QUIESCING', 'completion log precedes TIMEOUT release; reset contender acquires TIMEOUT before closing admission'),
            ('TERMINAL_QUARANTINE', 'FORWARD_PROGRESS', 'terminal log precedes TERMINAL release; progress observer acquires TERMINAL')]:
            aa = next((f'g{e["seq"]}' for e in guests if e['stage']==a), None)
            bb = next((f'g{e["seq"]}' for e in guests if e['stage']==b), None)
            if aa and bb: edges.append(dict(before=aa, after=bb, basis=basis))
        active_accesses = 0
        active_at_entry = None
        for event in devices:
            if event['stage']=='PAYLOAD_ACCESS_STARTED': active_accesses += 1
            if event['stage']=='PAYLOAD_ACCESS_ENDED': active_accesses -= 1
            if event['stage']=='RESET_ENTERED': active_at_entry=active_accesses
            assert active_accesses in (0,1)
        assert active_accesses==0
        assert active_at_entry in (None,0)
        # Validate graph acyclicity independently of host/guest clock origins.
        ids = {e['id'] for e in nodes}
        remaining = set(ids)
        while remaining:
            roots = {n for n in remaining if not any(e['after']==n and e['before'] in remaining for e in edges)}
            if not roots: raise AssertionError(('cyclic event graph',r['cpu'],r['mode']))
            remaining -= roots
        ack = next((i for i,e in enumerate(devices) if e['stage']=='RESET_ACKNOWLEDGED'), None)
        if ack is not None:
            positive = [i for i,e in enumerate(devices) if e['stage']=='POSITIVE_CONTROL_ACCESS_ENDED']
            assert len(positive)==1 and positive[0] < ack, ('positive control after claimed fence', r['serial'])
        violations = [e for i,e in enumerate(devices) if ack is not None and i>ack and
                      (e['stage'] in ['ILLEGAL_POST_FENCE_ACCESS','DEVICE_FETCHED'] or (e['stage'] in ['PAYLOAD_ACCESS_ENDED','POSITIVE_CONTROL_ACCESS_ENDED','GUEST_WITNESS_PUBLISHED'] and e['bytes']>0))]
        assert bool(violations) == (r['mode']=='negative')
        serial = (raw/r['serial']).read_text()
        identity = re.search(r'queue_incarnation: (\d+), generation: (\d+), head: (\d+)', next(line for line in serial.splitlines() if line.startswith('[SFS5B4] request=')))
        owner = next(e for e in guests if e['stage']=='RESET_OWNER_ESTABLISHED')
        request_identity = dict(device_incarnation=1, queue_incarnation=int(identity[1]), generation=int(identity[2]), head=int(identity[3]), lba=63)
        graphs.append(dict(active_payload_operations_at_reset_entry=active_at_entry, request_identity=request_identity,
                           reset_ownership=dict(terminal_epoch=1, owner_task_incarnation=owner['task_generation'], owner_cpu=owner['cpu'], queue_incarnation=request_identity['queue_incarnation']),
                           decision_events=dict(retirement='guest FENCE_UNQUALIFIED denies release; cancellation denies acknowledgment', new_epoch='guest ADMISSION_CLOSED refuses new admission; no reinitialization path'),
                           cpu=r['cpu'],cache=r['cache'],mode=r['mode'],nodes=nodes,edges=edges,acyclic=True,
                           violations=violations,expected_violation=r['mode']=='negative',
                           raw=str((raw/(r['serial'].replace('.serial.log','.events.json'))).relative_to(OUT))))
    put('device-event-graph.json',dict(status='PASS' if good else 'PARTIAL',model='INSTRUMENTED_QEMU + NATIVE_OBSERVED; separate nodes and explicit edges',
        timestamp_rule='No cross-clock subtraction; device ns monotonic within process only; guest counters do not establish cross-CPU program order',graphs=graphs))
    overlap = [
        ('O1','PASS','256-byte real mapped payload read before reset, explicit byte handshake precedes guest timeout'),
        ('O2','PASS','reset-read mode copies remaining 256 bytes inside reset handler before acknowledgment condition'),
        ('O3','UNQUALIFIED','BQL serializes payload copies and reset; no concurrent memory instruction witness'),
        ('O4','UNQUALIFIED','no async memory operation to drain; pending callback is not active RAM access'),
        ('O5','PARTIAL','scoped hook timer canceled, request detached, generic queues cleared; no general async/backend/platform fence'),
        ('O6','PASS','negative model performs actual host kernel pipe-copy read through original mapping after claimed fence; offline graph detects violation'),
        ('O7','PARTIAL','scalar-only late observer after reset; no original backend completion pending at cut'),
        ('O8','PASS','release/unmodified controls reset after legitimate completion, idle queue'),
        ('O9','PASS','held request at reset with no payload copy actively executing'),
        ('O10','PARTIAL','test-only error-cancel branch at reset; no backend EIO/transport failure injection'),
    ]
    put('dma-reset-overlap.json',dict(status='PARTIAL',native=rel,concurrency_model='B: deterministic BQL event ordering, asynchronous timer scheduling only',
        cases=[dict(case=c,status=s,observed=o,model='ADVERSARIAL_NEGATIVE' if c=='O6' else 'INSTRUMENTED_QEMU') for c,s,o in overlap],
        true_simultaneous_dma_reset='UNQUALIFIED',physical_dma='UNQUALIFIED'))
    put('negative-access-controls.json',dict(status='PASS' if all(r['checks']['negative_detector'] for r in rows) else 'FAIL',
        model='ADVERSARIAL_NEGATIVE',cases=len(neg),mechanism='post-claimed-fence successful 1-byte pipe-copy of still-pinned original payload mapping, before PROT_NONE',
        detector='device-event-graph.json violations scan on actual memory-operation events',
        protected_negative='five original host mappings protected; kernel pipe-copy returns EFAULT per page; positive control recorded before claimed acknowledgment and mprotect',
        limits='Host virtual CPU mappings, not IOMMU/physical DMA, aliases, hostile remapping or perfectly forged completions',
        production_control=False,native=rel))
    scenes = [('S1','NOT_RUN','atomic admission vs closing-store interleaving not forced'),('S2','NOT_RUN','real completion vs ownership cut not forced'),
              ('S3','NOT_RUN','timeout vs used-publication cut not forced'),('S4','PASS','two scheduled task incarnations compete for a live reset epoch; loser observes Closed, owner alone progresses'),
              ('S5','PARTIAL','acknowledgment followed by retained reconciliation; no release race'),('S6','PARTIAL','held timer canceled and scalar late observer; no active backend callback overlap'),
              ('S7','NOT_RUN','no native interrupt/queue-retirement cut; no queue retirement implemented'),('S8','PASS','I/O task observes admission closed during live terminal reset epoch'),
              ('S9','PARTIAL','explicit owner cancellation before reset keeps quarantine; involuntary worker exit and teardown integration NOT_RUN'),
              ('S10','PASS','after owner established, all online CPU timers and distinct pinned runnable workers advance before reset port write; no locks retained across yields')]
    smp=[]
    for r in rows:
        owners=[e for e in r['guest'] if e['stage']=='RESET_OWNER_ESTABLISHED']
        contenders=[e for e in r['guest'] if e['stage']=='RESET_OWNER_CONTESTED']
        workers=[e for e in r['guest'] if e['stage']=='INDEPENDENT_WORKER_RUNNING']
        assert owners and contenders
        assert sorted(set(e['cpu'] for e in workers)) == list(range(r['cpu']))
        assert r['cpu']==1 or owners[0]['cpu'] != contenders[0]['cpu']
        smp.append(dict(cpu=r['cpu'],cache=r['cache'],mode=r['mode'],status=r['status'],owners=owners,contenders=contenders,
            workers=workers,distinct_reset_cpus=bool(owners and contenders and owners[0]['cpu']!=contenders[0]['cpu']),
            independent_cpu_set=sorted(set(e['cpu'] for e in workers)),serial=r['serial'],
            simultaneous_device_memory_access='UNQUALIFIED'))
    put('smp-contention.json',dict(status='PARTIAL',native=rel,cases=[dict(case=c,status=s,scope=t) for c,s,t in scenes],witnesses=smp,
        qualification='Only S4/S8/S10 scoped PASS; 1CPU serial reference; no complete ten-race qualification'))
    put('forward-progress.json',dict(status='PASS' if good else 'PARTIAL',scope='bounded fixture progress during live reset ownership, explicit pre-reset work/tick rendezvous',
        native=rel,work_bound=2_000_000,host_boot_bound_seconds=90,reset_read_budget=64,
        lock_rule='try_lock coordinator; scheduler identity lock dropped before device step; each worker returns to ring3 and native yield',
        suspected_cycles='No production deadlock reproduced; prior polling exclusive owner prevented independent reset access. No scheduler repair claimed.',
        general_scheduler_fairness='UNQUALIFIED',arbitrary_block_backend_drain_wallclock='UNQUALIFIED'))
    put('scheduler-coordinator.json',dict(status='PARTIAL',implementation='kernel/src/sfs_reset.rs',fixture='kernel/src/sfs5b4_test.rs',feature='sfs5b4-test (off by default)',
        transitions=['RUNNING','QUIESCING','RESET_OWNER_ESTABLISHED','RESET_PENDING','RESET_ACKNOWLEDGED','FENCE_UNQUALIFIED','TERMINAL_QUARANTINE'],
        owner='CAS from zero to trusted task incarnation; attachment and queue generation in ticket; one terminal epoch, no owner reuse',
        admission='atomic closure with second check under attachment mutex; already accepted request inventoried before reset step takes same mutex',
        outstanding='one RequestId, retained first uncertain result, five pinned pages',
        cancellation='explicit cancel retains terminal DMA owner without reset acknowledgment; native cancel mode',
        missing='involuntary worker exit/reaper integration, native status-withholding and full completion/IRQ race cuts',native=rel))
    props=[('reset acknowledgment','PARTIAL','real native status-zero readback, instrumented handler/guard completion'),
           ('old-epoch access termination','PARTIAL','BQL scoped hook cancellation; no async engine'),('pending callbacks','PARTIAL','timer canceled, scalar observer only; arbitrary backend not qualified'),
           ('platform mapping lifetime','UNQUALIFIED','original host mappings only; no allocator reuse proof'),('queue ownership','PASS','terminal admission and old ring retention'),
           ('request/descriptor retirement','UNQUALIFIED','no retirement authority or reuse'),('persistence','UNQUALIFIED','write/flush uncertainty unaffected')]
    put('fencing-decision.json',dict(status='UNQUALIFIED',decision='FENCE_UNQUALIFIED; terminal quarantine',
        properties=[dict(property=p,status=s,evidence=e,models=(['UNQUALIFIED'] if p in ['platform mapping lifetime','request/descriptor retirement','persistence'] else ['NATIVE_OBSERVED','INSTRUMENTED_QEMU']), normative='conditional compliant-device contract only; no observed fence authorization') for p,s,e in props],model_A='NORMATIVE conforming-device queue interaction boundary; conditional platform assumptions',
        model_B='scoped serialized cancellation observed; no stronger asynchronous or platform retirement implication',model_C='negative post-fence access detected; broken model not safe'))
    inventories=[]
    for r in rows:
        text=(raw/r['serial']).read_text()
        memory=re.search(r'queue=(0x[0-9a-f]+) request=(0x[0-9a-f]+) pages=5',text)
        counts=re.search(r'free-before=(\d+) free-after=(\d+)',text)
        assert memory and counts
        assert counts[1] == counts[2], (r['serial'], counts.groups())
        qaddr,raddr=int(memory[1],16),int(memory[2],16)
        inventories.append(dict(cpu=r['cpu'],cache=r['cache'],mode=r['mode'],queue_pages=[qaddr+4096*i for i in range(4)],request_page=raddr,
            pinned_bytes=20480,released=0,free_before=int(counts[1]),free_after=int(counts[2]),pmm_unchanged=counts[1]==counts[2],
            uncertain_request_authorities=0 if r['mode'] in ['release','unmodified'] else 1,serial=r['serial']))
    put('dma-retirement.json',dict(inventories=inventories,status='PASS' ,scope='required fail-stop retention when fence is unqualified; retirement itself UNQUALIFIED',
        per_attachment=dict(queue_pages=4,request_pages=1,pinned_bytes=20480,request_authorities=1),
        released_pages=0,released_descriptors=0,retired_uncertain_requests=0,new_epochs=0,
        inventory_evidence='native serial inventory retains exact RequestId; PMM free counts sampled before/after; no per-frame reuse experiment',
        caveat='normal controls legitimately complete and retire normal requests before idle reset; five attachment pages still pinned',
        conditional_retirement='NOT_RUN',native=rel))
    put('queue-epoch.json',dict(status='UNQUALIFIED',old_epoch=1,new_epoch='NOT_RUN',fresh_post_reset_io='NOT_RUN',
        admission='terminally closed; no reset recovery/free API',late_completion='staged host tests retain timeout owner and refuse late ring; native completion-vs-reset race NOT_RUN',
        threat_limit='legacy wire used head/status cannot authenticate perfectly forged identical completions; software RequestId does not fix this',
        mitigations=['retain old resources while unresolved','fresh queue memory only after defensible old-epoch retirement/isolation','explicit trusted-device assumptions']))
    titles=['Baseline read/write/flush','Active request held','Payload access overlaps reset entry','Reset waits for active access','Acknowledgment after quiescence','Negative post-fence detector',
        'Reset timeout outstanding','Late device callback','Late completion after timeout','Reset racing completion','Concurrent reset requests','Admission racing terminal state',
        'Quarantine after failed fence','Conditional safe retirement','Repeated reset under pressure','New epoch','Fresh new-epoch I/O','Failed write provenance','Uncertain flush provenance',
        'Callback teardown','Device disappearance','2CPU progress','4CPU progress','Independent witness agreement']
    statuses=['PASS','PASS','UNQUALIFIED','UNQUALIFIED','PARTIAL','PASS','NOT_RUN','PARTIAL','NOT_RUN','NOT_RUN','PASS','PARTIAL','PASS','UNQUALIFIED','PARTIAL','NOT_RUN','NOT_RUN','PARTIAL','PARTIAL','PARTIAL','NOT_RUN','PARTIAL','PARTIAL','PASS']
    reasons=['real sector62 write/flush/readback and disposable release/unmodified controls','real 256-byte copy then outstanding used entry withheld',
        'reset-read copies inside serialized handler; no simultaneously active memory instruction','no asynchronous access operation to wait for',
        'scoped cancellation and queue clearing only','deliberate mapped CPU memory violation detected; not physical DMA',
        '64-read withholding fixtures HOST_MODEL only','scalar late observer; no original backend completion','late ring HOST_MODEL only','no native completion/reset race cut',
        'live owner and competing task/CPU Closed receipt','closed-admission probe observed; exact close/admit cut not forced','all unqualified outcomes retain five pages',
        'fence not qualified; no resources released','repeated begin/poll host fixtures; no native pressure sweep','old ownership unresolved','no new epoch',
        'native timed-out write uncertainty; failed-status provenance HOST_MODEL only','flush uncertainty HOST_MODEL only','held timer detach plus scalar late observer; arbitrary backend teardown unqualified',
        'not supported by current disposable harness','actual pinned CPU workers and reset contender; missing S1/S2/S3/S7 cuts','actual four-CPU workers and timer progress; incomplete race matrix',
        'explicit descriptor/payload status-byte and port-I/O edges; separate processes']
    matrix=[]
    for i,(title,status,reason) in enumerate(zip(titles,statuses,reasons),1):
        matrix.append(dict(case=f'N{i}',title=title,status=status,expected_interleaving=title,observed_interleaving=reason,
            instrumentation='raw serial, QEMU operation events, status-byte handshake, native scheduling and used-ring traces',
            guest_memory_access='real mapped descriptor/payload copies in scoped cases; simultaneous asynchronous DMA UNQUALIFIED',
            reset='real legacy port reset in active cases except explicit cancellation; HOST_MODEL coverage labeled separately',
            independent_evidence=[rel,'device-event-graph.json','smp-contention.json'],resource_accounting='five pages retained; zero guest DMA release',
            failure_behavior='bounded fixture failure, retained raw attempts and terminal quarantine',final_device_state='unavailable/quarantined; no new epoch'))
    put('native-fault-matrix.json',dict(status='PARTIAL',native=rel,cases=matrix,interleaving_rule='scoped observation does not qualify missing races'))
    put('durability-uncertainty.json',dict(status='PARTIAL',native=rel,uncertain_write='Completion::Uncertain, Amount::Unknown, Durability::Uncertain; first receipt retained across reset and closed admission',
        successful_controls='virtual flush/readback only; physical persistence UNQUALIFIED',failed_write='fresh HOST_MODEL driver error-provenance tests',
        uncertain_flush='fresh HOST_MODEL flush timeout/error tests; native reset cut NOT_RUN',reset_acknowledgment_is_durable_success=False))
    blockers=['earlier cpu-2 unmodified-QEMU early boot stall remains unexplained; see raw/early-boot-stall.json','safe true async guest-memory access/reset overlap absent under BQL model','arbitrary block-backend callback/drain quiescence not qualified',
        'platform mapping and allocator reuse proof missing','S1/S2/S3/S7 native cuts absent; S5/S6/S9 partial','involuntary reset worker exit/cancellation reaper integration absent',
        'native reset withholding/removal/late wire completions incomplete','native failed/uncertain flush reset cut missing','conditional retirement and new epoch intentionally NOT_RUN',
        'physical DMA fencing and hardware durability unqualified; native writable SunFS remains blocked']
    put('remaining-blockers.json',dict(status='PARTIAL',blockers=blockers,recommended_next='SFS-5B.5: reviewed AioContext/block-filter access lifetime backend and native completion/IRQ race cuts; retain quarantine until mapping/retirement proof'))
    put('production-changes.json',dict(status='PARTIAL',production=['sunlight-virtio/src/blk.rs: staged bounded write submit/poll/timeout and explicit terminal quarantine; synchronous methods share the same validation/completion logic'],
        diagnostic=['kernel/src/sfs_reset.rs','kernel/src/sfs5b4_test.rs','tools/helios-probes/sfs5b4-probe.S','sfs5b4-test feature, build, syscall and timer hooks'],
        default_transport='legacy split queue unchanged; no free/reinit/IOMMU path',claimed_deadlock_repair=False,
        preserved='all incoming source outside declared edits and all earlier SFS evidence verified separately by seal'))
    gates=dict(G1='PASS',G2='PARTIAL',G3='PARTIAL',G4='PASS' if good else 'PARTIAL',G5='UNQUALIFIED',G6='PASS' if good else 'PARTIAL',G7='PARTIAL',G8='PARTIAL',G9='PARTIAL',G10='PASS')
    put('validation-summary.json',dict(milestone='SFS-5B.4',status='PARTIAL',complete=False,scope='native SunlightOS in isolated QEMU11.1.2 TCG legacy PCI; BQL event ordering and post-scheduler tasks',
        gates=gates,native_scoped_boots=len(rows),native_scoped_assertions='PASS' if good else 'FAIL',native=rel,
        gate_limits=dict(G2='safe deferred/event-order backend only; no true async memory worker',G3='O1/O2 observed, O3/O4 unqualified',G4='independent witnesses for declared ordering scope only',
            G5='status zero and serialized cancellation do not qualify full retirement fence',G6='retention only; zero DMA retirement',G7='post-scheduler coordinator subset; involuntary exit/full races absent',
            G8='real S4/S8/S10 subset; remaining cuts absent',G9='available regressions recorded separately; full fault matrix incomplete',G10='artifact completeness and conservative statuses, not all gates complete'),
        dma_retirement='UNQUALIFIED',physical_hardware='UNQUALIFIED',native_writable_sunfs='BLOCKED',commit=False,push=False))
    print(rel,len(rows),good)


if __name__=='__main__': main()
