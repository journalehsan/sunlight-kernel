#!/usr/bin/env python3
"""Check evidence consistency without granting guest resource authority."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/audit/sunfs/sfs-5b6'
spec = importlib.util.spec_from_file_location('report', Path(__file__).with_name('sunfs5b6_report.py'))
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


def load(name):
    return json.loads((OUT / name).read_text())


def graph_valid(graph):
    nodes = graph['nodes']
    edges = graph['edges']
    if not report.acyclic(nodes, edges) or graph['missing_mandatory_edges']:
        return False
    required = [
        ('SUBMISSION_INITIATED', 'SUBMIT'),
        ('GUEST_HANDSHAKE', 'DEVICE_ACCESS_HANDSHAKE'),
        ('TIMEOUT_QUARANTINE', 'RESET_REQUESTED'),
        ('RESET_REQUESTED', 'DRAIN_BEGIN'),
    ]
    if any(node['stage'] == 'PRIVATE_OBSERVED_DRAIN' for node in nodes):
        required.extend([
            ('DRAIN_END', 'PRIVATE_OBSERVED_DRAIN'),
            ('PRIVATE_OBSERVED_DRAIN', 'PRIVATE_ACTIVE_AFTER_DRAIN'),
            ('PRIVATE_ACTIVE_AFTER_DRAIN', 'PRIVATE_EXIT'),
            ('PRIVATE_EXIT', 'PRIVATE_JOINED'),
        ])
    identities = {node['id']: node for node in nodes}
    stage_edges = {(identities[edge['before']]['stage'], identities[edge['after']]['stage']) for edge in edges}
    if not all(pair in stage_edges for pair in required):
        return False
    return any(identities[edge['before']]['viewpoint'] == 'device/backend' and
               identities[edge['after']]['viewpoint'] == 'guest' and
               identities[edge['before']]['stage'] == 'RESET_ACKNOWLEDGED' and
               identities[edge['after']]['stage'] == 'RESET_ACKNOWLEDGED' for edge in edges)


def main():
    required = [
        'source-baseline.json', 'backend-lifecycle-audit.md', 'real-backend-fixture.json',
        'backend-inflight-witness.json', 'drain-cancellation.json', 'callback-mapping-closure.json',
        'completion-races.json', 'interrupt-path-assessment.md', 'fresh-attachment-ownership.json',
        'allocator-retirement.json', 'allocator-reuse.json', 'historical-quarantine.json',
        'queue-epoch.json', 'native-fault-matrix.json', 'smp-progress.json',
        'independent-event-graph.json', 'forward-progress.json', 'durability-uncertainty.json',
        'remaining-blockers.json', 'regression-results.json', 'production-changes.json',
        'source-integrity.json', 'validation-summary.json',
    ]
    checks = {}
    sample = ('[SFS5B4] READY start-ticks=[10, 20] ticks=[14, 24] '
              'start-work=[100, 200] work=[105, 205]\n'
              '[SFS5B4] contention-start-ticks=[10, 20] ticks=[14, 24] '
              'contention-start-work=[100, 200] work=[105, 205]')
    counters = report.progress(sample, 2)
    progress_fields = ['timer_progress', 'task_progress', 'contention_timer_progress', 'contention_task_progress']
    checks['counter_prefix_parser_regression'] = all(counters[field] for field in progress_fields)
    missing_counters = report.progress('[SFS5B4] READY start-ticks=[10, 20] start-work=[100, 200]', 2)
    checks['missing_final_counters_rejected'] = not any(missing_counters[field] for field in progress_fields)
    checks['required_artifacts'] = all((OUT / name).is_file() for name in required)
    summary = load('validation-summary.json')
    fixture = load('real-backend-fixture.json')
    rows = [row for row in fixture['runs'] if row['final_guest'] and row['final_qemu']]
    backend = [row for row in rows if row['case'] not in ['control', 'indefinite']]
    profiles = {(cpu, cache) for cpu in [1, 2, 4] for cache in ['writeback', 'none']}
    checks['thirty_identity_bound_backend_runs'] = len(backend) == 30 and all(
        row['status'] == 'PASS' and row['callback_count'] == 1 and row['mapping_release_count'] == 1 and
        row['backend_admitted_inflight'] and row['backend_node_inflight'] and
        row['identity_agreement'] and row['physical_identity_agreement'] and
        row['ordered_closure'] and row['event_graph_closed'] and row['guest_timeout'] and
        (row['sector63_all_zero'] if row['case'] == 'error' else row['sector63_all_b4']) and
        (row['case'] == 'before' or row['drain_observed_inflight']) for row in backend)
    checks['six_profiles_each_case'] = all(
        {(row['cpu'], row['cache']) for row in backend if row['case'] == case} == profiles
        for case in ['before', 'during', 'cancel', 'error', 'private'])
    checks['native_private_smp_progress'] = all(all(row['smp_progress'][field] for field in progress_fields +
        ['all_workers_observed', 'ap_startup_verified', 'unique_reset_owner', 'reset_contested'])
        for row in backend if row['case'] == 'private')
    checks['separate_controls'] = len([row for row in rows if row['case'] == 'control' and
                                                      row['control_status'] == 'PASS']) == 6
    held = [row for row in rows if row['case'] == 'indefinite']
    checks['held_case_not_quiescence'] = len(held) == 1 and all(
        not row['ready'] and row['backend_admitted_inflight'] and row['drain_observed_inflight'] and
        row['callback_count'] == 0 and row['mapping_release_count'] == 0 and row['elapsed_s'] >= 45 for row in held)
    graphs = [graph for graph in load('independent-event-graph.json')['graphs']
              if graph['final_guest'] and graph['final_qemu'] and
              (graph['raw'], graph['run']) in {(row['raw'], row['tag']) for row in backend}]
    checks['mandatory_graph_edges'] = len(graphs) == 30 and all(graph_valid(graph) for graph in graphs)
    private = copy.deepcopy(next(graph for graph in graphs if any(
        node['stage'] == 'PRIVATE_OBSERVED_DRAIN' for node in graph['nodes'])))
    stages = {node['id']: node['stage'] for node in private['nodes']}
    private['edges'] = [edge for edge in private['edges'] if
                        (stages[edge['before']], stages[edge['after']]) != ('DRAIN_END', 'PRIVATE_OBSERVED_DRAIN')]
    checks['missing_sync_negative_rejected'] = not graph_valid(private)
    cyclic = copy.deepcopy(graphs[0])
    edge = cyclic['edges'][0]
    cyclic['edges'].append(dict(before=edge['after'], after=edge['before'], basis='negative control'))
    checks['cyclic_graph_negative_rejected'] = not graph_valid(cyclic)
    integrity = load('source-integrity.json')
    checks['prior_evidence_and_production_unchanged'] = integrity['status'] == 'PASS' and not integrity['prior_evidence_changed']
    checks['final_native_source_binding'] = True
    for directory in {row['raw'] for row in rows}:
        qemu = load(directory + '/qemu-source.json')
        checks['final_native_source_binding'] &= all(integrity['qemu_source'][name] == value for name, value in qemu.items())
        before = load(directory + '/source-before.json')
        after = load(directory + '/source-after.json')
        checks['final_native_source_binding'] &= all(
            name.startswith('tools/sunfs5b6_') or after.get(name) == value for name, value in before.items())
        invocation = next((OUT / directory).glob('*.invocation.json'))
        checks['final_native_source_binding'] &= report.runner.digest(OUT / directory / 'kernel.elf') == summary['final_guest_sha256']
        checks['final_native_source_binding'] &= report.runner.digest(OUT / directory / 'qemu-system-x86_64.snapshot') == summary['final_qemu_sha256']
        checks['final_native_source_binding'] &= json.loads(invocation.read_text())['qemu_sha256'] == summary['final_qemu_sha256']
    regressions = load('regression-results.json')
    checks['final_regressions'] = regressions['status'] == 'PASS' and summary['regression'] == 'PASS'
    archive = OUT / 'raw/rescued-hosted-release/raw-evidence.tar.gz'
    expected = {row['path']: row for profile in regressions['hosted_profiles'] if profile['profile'] == 'release'
                for row in profile['schedules']['files']}
    found = set()
    archive_ok = True
    with tarfile.open(archive, 'r|gz') as source:
        for member in source:
            relative = member.name.partition('/')[2]
            if relative not in expected or not member.isfile():
                continue
            contents = source.extractfile(member).read()
            rows_in_file = [json.loads(line) for line in contents.splitlines() if line]
            record = expected[relative]
            archive_ok &= hashlib.sha256(contents).hexdigest() == record['sha256']
            archive_ok &= len(rows_in_file) == record['count'] and all(row['status'] == 'PASS' for row in rows_in_file)
            found.add(relative)
    checks['release_raw_schedules_preserved'] = archive_ok and found == set(expected)
    checks['fresh_schedules_each_profile'] = all(profile['tests_passed'] == 54 and
        profile['schedules']['total'] == 13142 and all(row['all_pass'] for row in profile['schedules']['files'])
        for profile in regressions['hosted_profiles']) and len(regressions['hosted_profiles']) == 2
    debug = next(profile for profile in regressions['hosted_profiles'] if profile['profile'] == 'debug')
    debug_directory = (OUT / debug['log']).parent / 'debug'
    checks['debug_raw_schedules_preserved'] = all(
        report.runner.digest(debug_directory / row['path']) == row['sha256'] for row in debug['schedules']['files'])
    checks['no_guest_retirement_or_reuse'] = all(load(name)['guest_pages_released'] == 0 and
        load(name)['guest_pages_reused'] == 0 and load(name)['status'] == 'NOT_RUN'
        for name in ['allocator-retirement.json', 'allocator-reuse.json'])
    checks['no_epoch'] = not load('queue-epoch.json')['authorized'] and not summary['new_epoch_authorized']
    quarantine = load('historical-quarantine.json')
    checks['historical_quarantine_retained'] = quarantine['pages'] == 5 and quarantine['bytes'] == 20480 and quarantine['released'] == 0
    checks['honest_partial_verdict'] = summary['status'] == 'PARTIAL' and not summary['complete'] and summary['gates']['G7'] == 'NOT_RUN'
    output = dict(status='PASS' if all(checks.values()) else 'FAIL', checks=checks,
                  scope='evidence consistency only; parser negative controls do not qualify native allocator reuse')
    report.put(OUT / 'verification.json', output)
    print(json.dumps(output, indent=2), flush=True)
    if not all(checks.values()):
        raise SystemExit(1)
    manifest = {str(path.relative_to(ROOT)): report.runner.digest(path) for path in sorted(OUT.rglob('*'))
                if path.is_file() and path.name != 'artifact-manifest.json'}
    for path in [ROOT / 'tools/sunfs5b6_validate.py', ROOT / 'tools/sunfs5b6_report.py',
                 Path(__file__).resolve(), ROOT / 'docs/filesystems/sunfs/sfs-5b6-real-backend-drain-allocator-reuse.md']:
        manifest[str(path.relative_to(ROOT))] = report.runner.digest(path)
    report.put(OUT / 'artifact-manifest.json', dict(algorithm='sha256', files=manifest))


if __name__ == '__main__':
    main()
