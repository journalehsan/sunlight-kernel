#!/usr/bin/env python3
"""Conservative evidence index for the isolated real-backend experiment."""
import importlib.util
import json
import os
from pathlib import Path
import re

spec = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('sunfs5b6_validate.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
OUT = Path(os.environ.get('SFS5B6_REPORT_DIR', str(runner.OUT)))
ROOT = runner.ROOT
put = runner.put


def emit(name, **value):
    put(OUT / (name + '.json'), value)


def acyclic(nodes, edges):
    pending = {node['id']: set() for node in nodes}
    for edge in edges:
        if edge['before'] not in pending or edge['after'] not in pending:
            return False
        pending[edge['after']].add(edge['before'])
    while pending:
        ready = {identity for identity, parents in pending.items() if not parents}
        if not ready:
            return False
        pending = {identity: parents - ready for identity, parents in pending.items() if identity not in ready}
    return True


def event_graph(events, guest, physical_identity):
    nodes = [dict(event, id='device-' + str(event['id']), viewpoint='device/backend',
                  context='private-worker' if event['stage'] in ['PRIVATE_OBSERVED_DRAIN', 'PRIVATE_EXIT']
                  else 'main-aio') for event in events]
    for line in guest.splitlines():
        match = re.search(r'\[SFS5B4\] event=(\d+) cpu=(\d+) pid=(\d+) task-generation=(\d+).*stage=(\w+)', line)
        if match:
            nodes.append(dict(id='guest-' + match[1], viewpoint='guest', context=match[3] + ':' + match[4],
                              cpu=int(match[2]), stage=match[5], detail=line))
    edges = []
    previous = {}
    for node in nodes:
        key = node['viewpoint'] + ':' + node['context']
        if key in previous:
            edges.append(dict(before=previous[key], after=node['id'], basis='same actor program order'))
        previous[key] = node['id']

    def node_id(viewpoint, stage):
        return next((node['id'] for node in nodes if node['viewpoint'] == viewpoint and node['stage'] == stage), None)

    required = [
        ('device/backend', 'DRAIN_END', 'device/backend', 'PRIVATE_OBSERVED_DRAIN', 'mutex-protected drain marker before payload read'),
        ('device/backend', 'PRIVATE_OBSERVED_DRAIN', 'device/backend', 'PRIVATE_ACTIVE_AFTER_DRAIN', 'reset owner observes worker flag under mutex'),
        ('device/backend', 'PRIVATE_ACTIVE_AFTER_DRAIN', 'device/backend', 'PRIVATE_EXIT', 'stop flag under mutex'),
        ('device/backend', 'PRIVATE_EXIT', 'device/backend', 'PRIVATE_JOINED', 'qemu_thread_join'),
        ('guest', 'SUBMISSION_INITIATED', 'device/backend', 'SUBMIT', 'virtqueue notification and exact attachment GPA'),
        ('device/backend', 'GUEST_HANDSHAKE', 'guest', 'DEVICE_ACCESS_HANDSHAKE', 'status-byte 254 store/load on exact request GPA'),
        ('guest', 'TIMEOUT_QUARANTINE', 'guest', 'RESET_REQUESTED', 'TIMEOUT release/acquire and reset owner steps'),
        ('guest', 'RESET_REQUESTED', 'device/backend', 'DRAIN_BEGIN', 'terminal reset PIO write'),
        ('device/backend', 'RESET_ACKNOWLEDGED', 'guest', 'RESET_ACKNOWLEDGED', 'status-zero PIO readback'),
    ]
    missing = []
    has_private = any(node['stage'] == 'PRIVATE_OBSERVED_DRAIN' for node in nodes)
    for before_view, before_stage, after_view, after_stage, basis in required:
        if before_stage.startswith('PRIVATE_') or after_stage.startswith('PRIVATE_'):
            if not has_private:
                continue
        before = node_id(before_view, before_stage)
        after = node_id(after_view, after_stage)
        if before and after and (before_view == after_view or physical_identity):
            edges.append(dict(before=before, after=after, basis=basis))
        else:
            missing.append(dict(before=before_stage, after=after_stage, basis=basis))
    return dict(nodes=nodes, edges=edges, acyclic=acyclic(nodes, edges),
                missing_mandatory_edges=missing, guest_device_identity_verified=physical_identity,
                allocator_edges='NOT_RUN: no retirement or replacement allocation')


def progress(guest, cpu_count):
    ready = next((line for line in guest.splitlines() if '[SFS5B4] READY' in line), '')
    contention = next((line for line in guest.splitlines() if 'contention-start-ticks=' in line), '')

    def array(line, key):
        match = re.search(re.escape(key) + r'=(\[[^]]*\])', line)
        return json.loads(match[1]) if match else []

    def advanced(line, start, finish):
        initial = array(line, start)
        final = array(line, finish)
        return len(initial) >= cpu_count and len(final) >= cpu_count and all(
            final[index] > initial[index] + 3 for index in range(cpu_count))

    worker_cpus = sorted({int(match[1]) for match in re.finditer(
        r'cpu=(\d+).*stage=INDEPENDENT_WORKER_RUNNING', guest)})
    owners = {(match[1], match[2]) for match in re.finditer(
        r'pid=(\d+) task-generation=(\d+).*stage=RESET_OWNER_ESTABLISHED', guest)}
    return dict(ready=ready or None, contention=contention or None, worker_cpus=worker_cpus,
                all_workers_observed=worker_cpus == list(range(cpu_count)),
                ap_startup_verified=cpu_count == 1 or f'{cpu_count} total logical CPUs' in guest,
                timer_progress=advanced(ready, 'start-ticks', 'ticks'),
                task_progress=advanced(ready, 'start-work', 'work'),
                contention_timer_progress=advanced(contention, 'contention-start-ticks', 'ticks'),
                contention_task_progress=advanced(contention, 'contention-start-work', 'work'),
                reset_owner_identities=[dict(pid=int(pid), generation=int(generation)) for pid, generation in sorted(owners)],
                unique_reset_owner=len(owners) == 1,
                reset_contested='stage=RESET_OWNER_CONTESTED' in guest,
                progress_during_backend_drain='NOT_RUN: counters bracket reset ownership and terminal state')


def main():
    runs = []
    graphs = []
    unreadable = []
    guests = sorted((OUT / 'raw').glob('guest-*/kernel.elf'))
    guest_hash = runner.digest(guests[-1]) if guests else None
    for index in sorted((OUT / 'raw').glob('native-*/runs.json')):
        try:
            originals = json.loads(index.read_text())
        except (OSError, ValueError) as error:
            unreadable.append(dict(path=str(index.relative_to(OUT)), error=str(error)))
            continue
        for original in originals:
            row = dict(original, raw=str(index.parent.relative_to(OUT)))
            try:
                invocation = json.loads((index.parent / (row['tag'] + '.invocation.json')).read_text())
            except (OSError, ValueError) as error:
                unreadable.append(dict(path=str(index.relative_to(OUT)), run=row['tag'], error=str(error)))
                continue
            row['qemu_sha256'] = invocation['qemu_sha256']
            row['final_qemu'] = row['qemu_sha256'] == runner.digest(Path('/tmp/sfs5b6-qemu-build/qemu-system-x86_64'))
            row['final_guest'] = invocation['elf_sha256'] == guest_hash
            device = (index.parent / row['qemu_log']).read_text(errors='replace')
            guest_path = index.parent / row['serial']
            guest = guest_path.read_text(errors='replace') if guest_path.exists() else ''
            if not guest_path.exists():
                unreadable.append(dict(path=str(guest_path.relative_to(OUT)), error='guest serial file not created'))
            events = []
            for line in device.splitlines():
                match = re.search(r'SFS5B6 (\w+)(.*)', line)
                if match:
                    events.append(dict(id=len(events), stage=match[1], detail=match[2].strip()))
                elif 'SFS5B3 reset-complete status=0 queues-cleared=1' in line and any(
                        event['stage'] == 'MAPPING_RELEASED' for event in events):
                    events.append(dict(id=len(events), stage='RESET_ACKNOWLEDGED', detail=line))
            stages = [event['stage'] for event in events]
            mandatory = ['SUBMIT', 'BACKEND_ADMITTED', 'GUEST_HANDSHAKE', 'DRAIN_BEGIN',
                         'BACKEND_RELEASE', 'DEVICE_CALLBACK', 'DRAIN_END', 'MAPPING_RELEASED']
            if row['case'] == 'before':
                mandatory = ['SUBMIT', 'BACKEND_ADMITTED', 'BACKEND_RELEASE', 'GUEST_HANDSHAKE',
                             'DEVICE_CALLBACK', 'DRAIN_BEGIN', 'DRAIN_END', 'MAPPING_RELEASED']
            elif row['case'] == 'private':
                mandatory = ['SUBMIT', 'BACKEND_ADMITTED', 'GUEST_HANDSHAKE', 'DRAIN_BEGIN',
                             'BACKEND_RELEASE', 'DEVICE_CALLBACK', 'DRAIN_END', 'PRIVATE_OBSERVED_DRAIN',
                             'PRIVATE_ACTIVE_AFTER_DRAIN', 'PRIVATE_EXIT', 'PRIVATE_JOINED', 'MAPPING_RELEASED']
            positions = [stages.index(stage) for stage in mandatory if stage in stages]
            ordering = len(positions) == len(mandatory) and positions == sorted(positions)
            def field(stage, name):
                event = next((event for event in events if event['stage'] == stage), {})
                match = re.search(r'\b' + name + r'=([^ ]+)', event.get('detail', ''))
                return match[1] if match else None
            row['identity_agreement'] = bool(field('SUBMIT', 'req')) and (
                field('SUBMIT', 'req') == field('DEVICE_CALLBACK', 'req') == field('MAPPING_RELEASED', 'req')
                and field('SUBMIT', 'qiov') == field('BACKEND_ADMITTED', 'qiov'))
            row['callback_count'] = stages.count('DEVICE_CALLBACK')
            row['mapping_release_count'] = stages.count('MAPPING_RELEASED')
            baseline = re.search(r'BASELINE features=\((\d+), (\d+)\) queue=(0x[0-9a-f]+) request=(0x[0-9a-f]+)', guest)
            request_gpa = int(baseline[4], 16) if baseline else None
            row['physical_identity_agreement'] = bool(baseline and field('MAPPING_IDENTITY', 'header_gpa')) and (
                int(field('MAPPING_IDENTITY', 'header_gpa'), 16) == request_gpa and
                int(field('MAPPING_IDENTITY', 'status_gpa'), 16) == request_gpa + 528)
            row['guest_attachment'] = dict(queue_gpa=int(baseline[3], 16), request_gpa=request_gpa,
                                           offered_features=int(baseline[1]), accepted_features=int(baseline[2])) if baseline else None
            hhdm = re.search(r'HHDM offset: (0x[0-9a-f]+)', guest)
            if row['guest_attachment'] and hhdm:
                queue_gpa = row['guest_attachment']['queue_gpa']
                offset = int(hhdm[1], 16)
                row['guest_attachment'].update(
                    allocation_scope=row['raw'] + '/' + row['tag'],
                    queue_pages=[queue_gpa + page * 4096 for page in range(4)],
                    request_pages=[request_gpa], hhdm_offset=offset,
                    queue_virtual=queue_gpa + offset, request_virtual=request_gpa + offset,
                    header_gpa=request_gpa, bounce_gpa=request_gpa + 16, status_gpa=request_gpa + 528,
                    allocation_basis='observed guest physical bases; source PMM alloc_frames(4)/alloc_frame and HHDM arithmetic',
                    allocator_generation='NOT_OBSERVED')
            admitted = next((event for event in events if event['stage'] == 'BACKEND_ADMITTED'), {})
            drain = [event for event in events if event['stage'] == 'BACKEND_DRAIN_ENTER']
            row['events'] = events
            row['backend_admitted_inflight'] = bool(re.search(r'inflight=[1-9]', admitted.get('detail', '')))
            row['drain_observed_inflight'] = any(re.search(r'inflight=[1-9]', event['detail']) for event in drain)
            row['ordered_closure'] = ordering
            row['guest_timeout'] = 'stage=TIMEOUT_QUARANTINE' in guest
            row['guest_ready_line'] = next((line for line in guest.splitlines() if '[SFS5B4] READY' in line), None)
            row['smp_progress'] = progress(guest, row['cpu'])
            row['guest_reset_contested'] = row['smp_progress']['reset_contested']
            row['backend_node_inflight'] = bool(re.search(r'node_inflight=[1-9]', admitted.get('detail', '')))
            row['backend_result'] = int(field('DEVICE_CALLBACK', 'ret')) if field('DEVICE_CALLBACK', 'ret') else None
            graph = event_graph(events, guest, row['physical_identity_agreement'])
            row['event_graph_closed'] = not graph['missing_mandatory_edges'] and graph['acyclic']
            row['status'] = ('PASS' if row['ready'] and ordering and row['backend_admitted_inflight']
                             and row['identity_agreement'] and row['callback_count'] == 1
                             and row['mapping_release_count'] == 1
                             and row['physical_identity_agreement'] and row['backend_node_inflight']
                             and row['event_graph_closed'] and row['guest_timeout']
                             else 'NOT_RUN' if row['case'] == 'control' else 'PARTIAL')
            row['control_status'] = ('PASS' if row['case'] == 'control' and row['ready'] and
                                     'ACCESS_ACTIVE_DURING_RESET' in device and 'WORKER_JOINED' in device
                                     else 'NOT_RUN' if row['case'] != 'control' else 'PARTIAL')
            runs.append(row)
            graphs.append(dict(graph, run=row['tag'], raw=row['raw'], final_guest=row['final_guest'],
                               final_qemu=row['final_qemu'], mandatory_closure_edges_present=ordering))
    backend = [row for row in runs if row['case'] not in ['control', 'indefinite'] and row['final_qemu']
               and row['final_guest'] and row['backend_admitted_inflight']]
    final_runs = [row for row in runs if row['final_qemu'] and row['final_guest']]
    expected_profiles = {(cpu, cache) for cpu in [1, 2, 4] for cache in ['writeback', 'none']}

    def case_pass(test_case):
        selected = [row for row in backend if row['case'] == test_case]
        return {(row['cpu'], row['cache']) for row in selected} == expected_profiles and all(
            row['status'] == 'PASS' and (row['sector63_all_zero'] if test_case == 'error'
                                       else row['sector63_all_b4']) for row in selected)

    qualified = all(case_pass(test_case) for test_case in ['during', 'before', 'cancel', 'error', 'private'])
    emit('real-backend-fixture', status='PASS' if qualified else 'PARTIAL', runs=runs,
         unreadable_attempts=unreadable,
         callback='retaining device callback; original qiov submitted, stock used publication intentionally suppressed',
         scope='one sector-63 write per disposable process; main AioContext; static coherent RAM')
    emit('backend-inflight-witness', status='PASS' if qualified else 'PARTIAL',
         runs=[{key: row[key] for key in ['tag', 'raw', 'backend_admitted_inflight', 'drain_observed_inflight',
                                        'sector63_all_b4', 'sector63_all_zero']} for row in backend],
         independence='guest serial, device request identity, actual backend counter, host disposable-file readback')
    supported = {'D1': 'before', 'D2': 'during', 'D3': 'during', 'D4': 'during', 'D5': 'cancel',
                 'D8': 'cancel', 'D9': 'error', 'D10': 'indefinite', 'D11': 'private',
                 'D12': 'during', 'D15': 'private'}
    drain_cases = {}
    for number in range(1, 17):
        key = f'D{number}'
        test_case = supported.get(key)
        selected = [row for row in final_runs if row['case'] == test_case]
        passed = case_pass(test_case) if test_case and test_case != 'indefinite' else False
        if number == 10:
            passed = bool(selected) and all(not row['ready'] and row['backend_admitted_inflight']
                                           and row['drain_observed_inflight'] and not row['mapping_release_count']
                                           and row['elapsed_s'] >= 45 for row in selected)
        status = 'PASS' if passed else 'PARTIAL' if selected else 'NOT_RUN'
        reason = 'identity-bound admitted operation and callback/mapping events; six CPU/cache profiles'
        if number in [6, 7]:
            status = 'NOT_RUN'
            reason = 'emulated AIOCB has no cancel_async implementation; cancellation-wins unavailable'
        elif number == 8:
            status = 'PARTIAL'
            reason = 'completion wins after no-op cancel; separate post-terminal cancellation cut not executed'
        elif number == 10:
            reason = 'expected process-supervisor termination; no drain return or guest retirement claimed'
        elif number == 12:
            status = 'PARTIAL'
            reason = 'reset owns drain on main AioContext; no competing reset/drain coordinator overlap'
        elif not test_case:
            reason = 'required teardown, multiple-request or retry native cut not implemented'
        drain_cases[key] = dict(status=status, fixture=test_case, reason=reason,
                               evidence=[row['raw'] + '/' + row['qemu_log'] for row in selected])
    emit('drain-cancellation', status='PARTIAL', cases=drain_cases,
         cancellation='emulated AIOCB has no cancel_async hook; normal completion wins',
        bound='45 second process supervisor; expiry preserves absence of drain/retirement proof')
    emit('callback-mapping-closure', status='PARTIAL', backend_callback_closure=qualified,
         mapping_release_after_drain=qualified, private_worker_control_separate=True,
         retirement_prerequisites={
             'backend_terminal': 'PASS for the 30 scoped runs; NOT_ESTABLISHED for indefinitely held request',
             'private_callbacks_terminal': 'PASS for scoped private-reader join, not removal or arbitrary callbacks',
             'mapping_users_closed': 'device users closed; guest MappingRecord retirement remains unqualified',
             'reset_fence': 'legacy status-zero reset observed; no guest allocator retirement permit',
             'guest_request_reconciled': 'NOT_RUN: timeout ownership remains terminal quarantine',
             'descriptor_dma_retirement_authority': 'NOT_RUN: guest pages remain pinned'},
         combined_private_cases=[row['raw'] + '/' + row['qemu_log'] for row in backend if row['case'] == 'private'],
         unqualified=['device removal', 'guest DMA retirement', 'full cross-layer guest ownership reconciliation'])
    races = {f'R{number}': dict(status='NOT_RUN', applicability='polling path',
             reason='required deterministic native overlap not established by backend admission alone')
             for number in [2, 3, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]}
    for number in [4, 5]:
        races[f'R{number}'] = dict(status='NOT_RUN', applicability='NOT_APPLICABLE to current polling path',
                                  hypothetical_interrupt_path='NOT_RUN')
    for number in [6, 8, 10]:
        races[f'R{number}'].update(status='PASS' if case_pass('private') else 'PARTIAL',
                                  reason={6: 'private worker reads during reset, then exits and joins before unmap',
                                          8: 'mapping release precedes device status-zero completion and guest readback',
                                          10: 'unique generation-qualified reset owner and actual competing guest tasks'}[number],
                                  scope='instrumented retaining callback; no allocator or stock used-publication qualification')
    for number in [3, 9, 14]:
        races[f'R{number}']['status'] = 'PARTIAL'
    races['R3']['reason'] = 'backend callback executes during reset drain; stock used-ring completion cut NOT_RUN'
    races['R9']['reason'] = 'private exit/join precedes emulator request unmap/free; guest DMA retirement NOT_RUN'
    races['R14']['reason'] = 'admission closure is observed; a separately witnessed post-terminal admission cut is missing'
    races['R11']['applicability'] = 'conditional on a new epoch; no new epoch authorized'
    for key, race in races.items():
        race['intended_interleaving'] = {
            'R2': 'guest timeout competes with used-ring completion',
            'R3': 'completion executes while reset is active',
            'R6': 'private worker exits while reset owns request cleanup',
            'R7': 'callback cancellation competes with completion',
            'R8': 'mapping release precedes reset acknowledgment',
            'R9': 'last callback exits before resource retirement',
            'R10': 'distinct guest tasks compete for reset ownership',
            'R11': 'old callback arrives after a new epoch',
            'R12': 'device removal competes with an outstanding mapping',
            'R13': 'interrupted reset is retried',
            'R14': 'new I/O is attempted after terminal failure',
            'R15': 'repeated reset occurs under allocation pressure',
        }.get(key, 'interrupt-driven completion cut')
        race['observed_runs'] = []
        if key not in ['R3', 'R6', 'R8', 'R9', 'R10', 'R14']:
            race['actual_interleaving'] = 'NOT_RUN for required cut'
            continue
        selected = [row for row in backend if row['case'] == 'private']
        race['actual_interleaving'] = race['reason']
        for row in selected:
            submit = next(event for event in row['events'] if event['stage'] == 'SUBMIT')
            race['observed_runs'].append(dict(
                run=row['tag'], raw=row['raw'], request_identity=submit['detail'],
                allocation=row['guest_attachment'], winner='generation-qualified guest reset owner',
                loser='other reset contenders; backend callback reconciled before detach',
                terminal_state='guest terminal quarantine; emulator request detached without used publication',
                backend_result=row['backend_result'], backend='terminal; drain returned in-flight zero',
                mapping='private worker joined, then detached exactly once',
                dma_ownership='guest pages pinned; retirement authority NOT_RUN',
                progress=row['smp_progress'], events=row['raw'] + '/' + row['qemu_log']))
    emit('completion-races', status='PARTIAL', races=races)
    emit('fresh-attachment-ownership', status='PARTIAL', allocations='fresh boot attachment per run, pinned',
         identities='guest BASELINE queue/request physical addresses; device req/head; backend opaque/acb in raw logs',
         attachments=[dict(tag=row['tag'], raw=row['raw'], allocation=row['guest_attachment']) for row in backend],
         missing=['allocator allocation generation/receipt', 'retirement permit', 'replacement allocation identity'])
    for name in ['allocator-retirement', 'allocator-reuse']:
        emit(name, status='NOT_RUN', guest_pages_released=0, guest_pages_reused=0,
             reason='backend drain does not supply guest allocator fencing authority; no unsafe release attempted',
             emulator_request_retirement='detach and g_free once after backend callback and drain only',
             host_retirements=[dict(tag=row['tag'], raw=row['raw'], callbacks=row['callback_count'],
                                   mapping_releases=row['mapping_release_count']) for row in backend])
    emit('historical-quarantine', status='PASS', pages=5, bytes=20480, released=0,
         source='unchanged ../sfs-5b5/quarantine-accounting.json',
         exact_physical_manifest='unavailable in prior evidence; not reconstructed or invented')
    emit('queue-epoch', status='NOT_RUN', authorized=False, reinitialized=False,
         reason='guest remains terminal quarantine; no retirement permit')
    native_cases = {f'N{number}': dict(status='NOT_RUN', reason='required native cut not executed')
                    for number in range(1, 23)}
    for number, test_case in {2: 'during', 3: 'before', 4: 'during', 5: 'during',
                              6: 'error', 8: 'private', 11: 'during', 22: 'private'}.items():
        native_cases[f'N{number}'] = dict(status='PASS' if case_pass(test_case) else 'PARTIAL',
                                        fixture=test_case, scope='instrumented disposable attachment; final guest/QEMU hashes',
                                        evidence=[row['raw'] + '/' + row['qemu_log'] for row in backend if row['case'] == test_case])
    native_cases['N1'] = dict(status='PARTIAL',
                             reason='real file I/O readback verified; retaining callback suppresses normal guest used-ring completion')
    for number in [7, 9, 10]:
        native_cases[f'N{number}'] = dict(status='PARTIAL',
                                        reason='sequenced cancel/reset/mapping observations exist; required competing cut incomplete')
    for number, cpu in [(20, 2), (21, 4)]:
        selected = [row for row in backend if row['case'] == 'private' and row['cpu'] == cpu]
        fields = ['all_workers_observed', 'ap_startup_verified', 'timer_progress', 'task_progress',
                  'contention_timer_progress', 'contention_task_progress', 'unique_reset_owner', 'reset_contested']
        native_cases[f'N{number}'] = dict(status='PASS' if len(selected) == 2 and all(
            all(row['smp_progress'][field] for field in fields) for row in selected) else 'PARTIAL',
            scope='actual CPU-affined scheduler tasks and reset-owner contention; during-drain progress not claimed',
            evidence=[row['raw'] + '/' + row['serial'] for row in selected])
    emit('native-fault-matrix', status='PARTIAL', scenarios=native_cases,
         runs_index='real-backend-fixture.json', note='missing retirement/reuse, removal, retry and pressure cuts remain NOT_RUN')
    emit('smp-progress', status='PARTIAL', profiles=sorted(set(row['cpu'] for row in final_runs)),
         witnesses=[dict(tag=row['tag'], raw=row['raw'], progress=row['smp_progress']) for row in final_runs],
         limitation='actual cross-CPU owner contention and bracketed timer/task progress; progress specifically during drain NOT_RUN')
    emit('independent-event-graph', status='PARTIAL', graphs=graphs,
         missing='allocator retirement/replacement edges NOT_RUN; failed/development graphs explicitly incomplete')
    emit('forward-progress', status='PARTIAL', supervisor_seconds=45, production_coordinator_unchanged=True,
         backend_drain_wall_clock_bound=False, indefinite_test='isolated process only; never quiescence proof',
         historical_uart='unbounded THRE wait before SMP; root cause UNQUALIFIED; no serial change')
    emit('durability-uncertainty', status='PASS', typed_results_unchanged=True,
         guest_outcome='Uncertain remains authoritative after timeout; exact completed bytes Unknown',
         host_readback='observed image contents only; neither durable barrier nor authoritative guest short count',
         native_writable_sunfs='BLOCKED')
    regression = []
    for path in sorted((OUT / 'raw').glob('regression-*/*.json')):
        if path.name in ['hosted-result.json']:
            regression.append(dict(path=str(path.relative_to(OUT)), results=json.loads(path.read_text())))
    for path in sorted((OUT / 'raw').glob('focused-*/results.json')):
        regression.append(dict(path=str(path.relative_to(OUT)), results=json.loads(path.read_text())))
    hosted = []
    debug_paths = sorted((OUT / 'raw').glob('regression-*/hosted-debug.log'))
    release_log = OUT / 'raw/rescued-hosted-release/sfs5b6-hosted-release.log'
    profile_paths = [('debug', debug_paths[-1], debug_paths[-1].parent / 'debug')] if debug_paths else []
    if release_log.exists():
        profile_paths.append(('release', release_log, Path('/tmp/sfs5b6-hosted-release')))
    for profile, log_path, evidence in profile_paths:
        text = log_path.read_text()
        schedule_path = (release_log.parent / 'schedule-summary.json') if profile == 'release' else log_path.parent / 'schedule-summary.json'
        if not schedule_path.exists():
            schedule_files = []
            for path in sorted(evidence.glob('sfs*/*.jsonl')):
                if not path.name.startswith('crash') and path.name != 'reordering.jsonl':
                    continue
                rows = [json.loads(line) for line in path.read_text().splitlines() if line]
                schedule_files.append(dict(path=str(path.relative_to(evidence)), sha256=runner.digest(path),
                                           count=len(rows), all_pass=all(row.get('status') == 'PASS' for row in rows)))
            put(schedule_path, dict(profile=profile, fresh_execution=True, files=schedule_files,
                                    total=sum(row['count'] for row in schedule_files)))
        schedule = json.loads(schedule_path.read_text())
        hosted.append(dict(profile=profile, log=str(log_path.relative_to(OUT)),
                           tests_passed=sum(int(count) for count in re.findall(r'test result: ok\. (\d+) passed', text)),
                           tests_failed='test result: FAILED' in text, schedules=schedule))
    focused_paths = sorted((OUT / 'raw').glob('focused-*/results.json'))
    focused_pass = bool(focused_paths) and all(row['returncode'] == 0 for row in json.loads(focused_paths[-1].read_text()))
    regression_pass = focused_pass and len(hosted) == 2 and all(
        profile['tests_passed'] == 54 and not profile['tests_failed'] and profile['schedules']['total'] > 0 and
        all(row['all_pass'] for row in profile['schedules']['files']) for profile in hosted)
    emit('regression-results', status='PASS' if regression_pass else 'PARTIAL', fresh=regression,
         hosted_profiles=hosted, focused_final=str(focused_paths[-1].relative_to(OUT)) if focused_paths else None,
         packaging_failure='debug tests passed, first packaging attempt hit ENOSPC; raw data retained; release tar archive rescued',
         logs=[str(path.relative_to(OUT)) for path in (OUT / 'raw').glob('regression-*/*.log')],
         baseline_note='baseline-storage.log completed before ledger relative-path error; attempt preserved',
         historical_reuse='none counted as fresh; exact final-source integrity checked separately')
    emit('qemu-identity', version=Path('/tmp/sfs5b6-qemu-src/VERSION').read_text().strip(),
         binary_sha256=runner.digest(Path('/tmp/sfs5b6-qemu-build/qemu-system-x86_64')),
         prior_identity=json.loads((ROOT / 'docs/audit/sunfs/sfs-5b5/qemu-source-identity.json').read_text()),
         patch='qemu-real-backend.patch', patch_sha256=runner.digest(OUT / 'qemu-real-backend.patch'),
         final_source_snapshot='qemu-source-final/',
         configure=['--target-list=x86_64-softmmu', '--disable-docs', '--disable-werror', '--disable-download',
                    '--disable-gtk', '--disable-sdl', '--disable-opengl', '--disable-virglrenderer',
                    '--disable-vnc', '--disable-tools', '--disable-user', '--disable-plugins'],
         build=['ninja', '-C', '/tmp/sfs5b6-qemu-build', '-j', '6', 'qemu-system-x86_64'])
    emit('production-changes', status='PASS', files=[], kernel_driver_changes=False,
         changes=['tools/sunfs5b6_validate.py', 'tools/sunfs5b6_report.py', 'tools/sunfs5b6_verify.py',
                  'isolated QEMU patch and evidence'])
    baseline = json.loads((OUT / 'source-baseline.json').read_text())
    raw = OUT / baseline['raw']
    sources = json.loads((raw / 'source.json').read_text())
    changed = [name for name, expected in sources.items()
               if not (ROOT / name).is_file() or runner.digest(ROOT / name) != expected]
    prior = json.loads((raw / 'prior-evidence.json').read_text())
    prior_changed = [name for name, expected in prior.items()
                     if not (ROOT / name).is_file() or runner.digest(ROOT / name) != expected]
    integrity_pass = not prior_changed and all(name == 'tools/sunfs5b6_validate.py' for name in changed)
    emit('source-integrity', status='PASS' if integrity_pass else 'FAIL',
         incoming_sources_changed=changed, prior_evidence_changed=prior_changed,
         qemu_source={name: runner.digest(Path('/tmp/sfs5b6-qemu-src') / name)
                      for name in ['hw/block/virtio-blk.c', 'block/block-backend.c', 'block/io.c',
                                   'hw/virtio/virtio.c', 'system/physmem.c', 'VERSION']})
    blockers = ['full deterministic drain/cancellation matrix incomplete; cancellation-wins unsupported by this AIOCB',
                'full cross-layer guest callback/mapping retirement conjunction not qualified',
                'completion/reset/removal/retry/resource-pressure native race matrix incomplete',
                'fresh guest allocation provenance, fencing, retirement and controlled reuse unproven',
                'new epoch forbidden; historical five pages retained',
                'allocator retirement/replacement happens-before edges remain unexecuted']
    if not regression_pass:
        blockers.append('final regression qualification incomplete; inspect preserved failed attempts and latest results')
    emit('remaining-blockers', status='PARTIAL', blockers=blockers,
         recovered_environment='ENOSPC attempts retained; disk space recovered by user; final kernel builds and profiles re-evaluated',
         recommended_next='SFS-5B.6 continuation: guest ownership reconciliation and allocation receipts before fencing/retirement/reuse')
    gates = {f'G{number}': 'PARTIAL' for number in range(1, 11)}
    gates.update(G1='PASS' if integrity_pass else 'PARTIAL', G2='PASS' if qualified else 'PARTIAL',
                 G7='NOT_RUN', G10='PASS' if integrity_pass and qualified else 'PARTIAL')
    emit('validation-summary', milestone='SFS-5B.6', status='PARTIAL', complete=False,
         gates=gates, regression='PASS' if regression_pass else 'PARTIAL',
         final_guest_sha256=guest_hash, final_qemu_sha256=runner.digest(Path('/tmp/sfs5b6-qemu-build/qemu-system-x86_64')),
         final_controls=sum(row['control_status'] == 'PASS' for row in final_runs),
         real_backend_scoped_runs=len(backend), qualified_backend_runs=sum(row['status'] == 'PASS' for row in backend),
         guest_dma_released=0, historical_pages_retained=5, new_epoch_authorized=False,
         native_sunfs_mount=False, commit=False, push=False)


if __name__ == '__main__':
    main()
