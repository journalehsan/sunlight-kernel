#!/usr/bin/env python3
"""Regenerate reviewable per-trial CSV and pooled C1.2E evidence summaries."""
import argparse
import csv
import json
from pathlib import Path
import statistics as st

from scheduler_fairness import validate_suite
from scheduler_service_compare import services
from scheduler_timer import analyze, quantiles


def distribution(values):
    return dict(n=len(values), mean=st.mean(values), sd=st.pstdev(values), min=min(values), max=max(values))


def collect(folder):
    records = []
    for variant in ('baseline-repeat1', 'baseline-repeat2', 'before-trace', 'after-trace',
                    'before-performance', 'after-performance', 'before-mixed-paced', 'after-mixed'):
        directory = folder / variant
        paths = sorted(directory.glob('capture*/1cpu-run1.serial.log')) if not variant.startswith('baseline-') else [directory / '1cpu-run1.serial.log']
        for path in paths:
            raw = path.read_text(errors='replace')
            mixed = 'mixed' in variant
            traced = mixed or variant.endswith('trace')
            if traced:
                measured = analyze(raw, mixed)
                path.with_name('timer.json').write_text(json.dumps(measured, indent=2) + '\n')
                for r in measured['runs']:
                    records.append(dict(variant=variant, capture=path.parent.name, phase='startup' if r['startup'] else 'steady', **r))
            else:
                suite = validate_suite(raw)
                service_records = services(raw, suite['runs'])
                for r in suite['runs']:
                    service = next(s for s in service_records if s['run'] == f'1-{r["run"]}-1' and s['name'] == 'timer_server')
                    records.append(dict(variant=variant, capture=path.parent.name, phase='startup' if r['run'] <= 4 else 'steady',
                        run=r['run'], duration_ms=r['duration_ms'], order=r['order'], nice=[w['nice'] for w in r['workers']],
                        cpu_ns=[w['cpu_ns'] for w in r['workers']], cpu_jain=r['cpu_fairness']['jain'],
                        work_jain=r['work_fairness']['jain'], operations=[w['operations'] for w in r['workers']],
                        ops_per_second=sum(w['operations'] for w in r['workers']) * 1000 / r['elapsed_ms'],
                        ops_per_worker_cpu_second=sum(w['operations'] for w in r['workers']) * 1e9 / sum(w['cpu_ns'] for w in r['workers']),
                        worker_dispatches=sum(w['dispatches'] for w in r['workers']), timer_cpu_ns=service['cpu_ns'], timer_dispatches=service['dispatches'],
                        series=dict(ready_to_dispatch=service['waits_ns']), replies=[], audio=[]))
    return records


def summarize(records):
    keys = sorted({(r['variant'], r['phase'], r['duration_ms'], r['order'], tuple(r['nice'])) for r in records})
    result = []
    for variant, phase, duration, order, nice in keys:
        members = [r for r in records if (r['variant'], r['phase'], r['duration_ms'], r['order'], tuple(r['nice'])) == (variant, phase, duration, order, nice)]
        series_keys = sorted({key for r in members for key in r['series']})
        replies = [reply for r in members for reply in r['replies']]
        saturated = [r for r in replies if r['request_saturated'] and r['complete_saturated']]
        result.append(dict(variant=variant, phase=phase, duration_ms=duration, order=order, nice=nice,
            cpu_jain=distribution([r['cpu_jain'] for r in members]), ops_per_second=distribution([r['ops_per_second'] for r in members]),
            work_jain=distribution([r['work_jain'] for r in members]),
            ops_per_worker_cpu_second=distribution([r['ops_per_worker_cpu_second'] for r in members]),
            worker_dispatches=distribution([r['worker_dispatches'] for r in members]),
            timer_cpu_ns=distribution([r['timer_cpu_ns'] for r in members]), timer_dispatches=distribution([r['timer_dispatches'] for r in members]),
            latency={key: quantiles([ns for r in members for ns in r['series'].get(key, [])]) for key in series_keys},
            saturated_rpc=quantiles([r['complete'] - r['request'] for r in saturated]),
            saturated_request_count=sum(r['request_saturated'] for r in replies),
            cpu_ns_by_slot=[[r['cpu_ns'][i] for r in members] for i in range(4)],
            audio=[a for r in members for a in r['audio']],
            per_boot={capture: dict(cpu_jain=st.mean(r['cpu_jain'] for r in members if r['capture'] == capture),
                ops_per_second=st.mean(r['ops_per_second'] for r in members if r['capture'] == capture),
                latency={key: quantiles([ns for r in members if r['capture'] == capture for ns in r['series'].get(key, [])]) for key in series_keys})
                for capture in sorted({r['capture'] for r in members})}))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence-dir', type=Path, default=Path('target/scheduler-c12e'))
    parser.add_argument('--csv', type=Path, required=True)
    args = parser.parse_args()
    records = collect(args.evidence_dir)
    summary = summarize(records)
    (args.evidence_dir / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    columns = ['variant', 'capture', 'run', 'phase', 'duration_ms', 'order', 'nice', 'cpu_jain', 'work_jain', 'ops_per_second', 'cpu_ns', 'operations',
               'ops_per_worker_cpu_second', 'worker_dispatches', 'timer_cpu_ns', 'timer_dispatches',
               'timer_wait_n', 'timer_wait_p50_ms', 'timer_wait_p95_ms', 'timer_wait_p99_ms', 'timer_wait_max_ms',
               'rpc_n', 'rpc_p50_ms', 'rpc_p95_ms', 'rpc_p99_ms', 'rpc_max_ms']
    with args.csv.open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=columns)
        writer.writeheader()
        for r in records:
            row = {key: r[key] for key in columns if key in r}
            row['nice'], row['cpu_ns'] = json.dumps(row['nice']), json.dumps(row['cpu_ns'])
            row['operations'] = json.dumps(row['operations'])
            for label, key in [('timer_wait', 'ready_to_dispatch'), ('rpc', 'request_to_completion')]:
                q = quantiles(r['series'].get(key, []))
                row[label + '_n'] = q['n']
                for p in (50, 95, 99, 100):
                    row[label + ('_max_ms' if p == 100 else f'_p{p}_ms')] = q.get(f'p{p}_ms', '')
            writer.writerow(row)
    print(f'{len(records)} validated trials; {len(summary)} pooled groups')


if __name__ == '__main__':
    main()
