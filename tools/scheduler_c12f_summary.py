#!/usr/bin/env python3
"""Regenerate validated C1.2F per-trial and pooled performance evidence."""
import argparse
import csv
import json
from pathlib import Path
import re
import statistics as st

from scheduler_fairness import validate_suite
from scheduler_service_compare import services
from scheduler_timer import analyze, quantiles


def collect(root):
    result = []
    for variant in ('before-trace', 'notify-trace', 'final-trace',
                    'before-performance-cost', 'final-performance', 'before-mixed-retry', 'final-mixed'):
        for path in sorted((root / variant).glob('capture*/1cpu-run1.serial.log')):
            raw = path.read_text(errors='replace')
            mixed = 'mixed' in variant
            traced = mixed or variant.endswith('trace')
            cost_rows = [json.loads(v) for v in re.findall(r'^\[SCHED-BENCH\] COST (.*)$', raw, re.M)]
            costs = {v['run']: v for v in cost_rows}
            if len(costs) != len(cost_rows) or any(v['context_switches'] <= 0 or v['elapsed_ns'] <= 0 for v in cost_rows):
                raise ValueError('invalid/duplicate context-switch counters')
            if 'performance' in variant and len(costs) != 22:
                raise ValueError('missing performance context-switch counters')
            if traced:
                records = analyze(raw, mixed)['runs']
            else:
                runs = validate_suite(raw)['runs']
                sr = services(raw, runs)
                records = []
                for r in runs:
                    timer = next(s for s in sr if s['run'] == f'1-{r["run"]}-1' and s['name'] == 'timer_server')
                    records.append(dict(run=r['run'], startup=r['run'] <= 4, duration_ms=r['duration_ms'], order=r['order'],
                        nice=[w['nice'] for w in r['workers']], cpu_ns=[w['cpu_ns'] for w in r['workers']],
                        cpu_jain=r['cpu_fairness']['jain'], ops_per_second=sum(w['operations'] for w in r['workers'])*1000/r['elapsed_ms'],
                        worker_dispatches=sum(w['dispatches'] for w in r['workers']), timer_dispatches=timer['dispatches'],
                        series=dict(ready_to_dispatch=timer['waits_ns'])))
            for r in records:
                c = costs.get(f'1-{r["run"]}-1')
                result.append(dict(variant=variant, capture=path.parent.name, **r,
                    context_switches=c['context_switches'] if c else None,
                    context_switches_per_second=c['context_switches']*1e9/c['elapsed_ns'] if c else None))
    return result


def summary(records):
    groups = []
    keys = sorted({(r['variant'], r['startup'], r['duration_ms'], r['order'], tuple(r['nice'])) for r in records})
    for variant, startup, duration, order, nice in keys:
        rs = [r for r in records if (r['variant'], r['startup'], r['duration_ms'], r['order'], tuple(r['nice'])) == (variant, startup, duration, order, nice)]
        groups.append(dict(variant=variant, startup=startup, duration_ms=duration, order=order, nice=nice, trials=len(rs),
            cpu_jain_mean=st.mean(r['cpu_jain'] for r in rs), cpu_jain_min=min(r['cpu_jain'] for r in rs),
            ops_per_second_mean=st.mean(r['ops_per_second'] for r in rs),
            worker_dispatches_mean=st.mean(r['worker_dispatches'] for r in rs),
            context_switches_per_second_mean=st.mean(r['context_switches_per_second'] for r in rs if r['context_switches_per_second'] is not None) if any(r['context_switches'] for r in rs) else None,
            latency={key: quantiles([v for r in rs for v in r['series'].get(key, [])]) for key in sorted({k for r in rs for k in r['series']})}))
    return groups


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence-dir', type=Path, default=Path('target/scheduler-c12f'))
    parser.add_argument('--csv', type=Path, required=True)
    args = parser.parse_args()
    records = collect(args.evidence_dir)
    (args.evidence_dir / 'summary.json').write_text(json.dumps(summary(records), indent=2)+'\n')
    columns = ['variant', 'capture', 'run', 'startup', 'duration_ms', 'order', 'nice', 'cpu_ns', 'cpu_jain', 'ops_per_second',
               'worker_dispatches', 'timer_dispatches', 'context_switches', 'context_switches_per_second',
               'wake_n', 'wake_p50_ms', 'wake_p95_ms', 'wake_p99_ms', 'wake_max_ms', 'censored_wakes']
    with args.csv.open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, lineterminator='\n')
        writer.writeheader()
        for r in records:
            row = {k: r[k] for k in columns if k in r}
            row['nice'], row['cpu_ns'] = json.dumps(row['nice']), json.dumps(row['cpu_ns'])
            q = quantiles(r['series'].get('wake_to_dispatch', []))
            row['wake_n'] = q['n']
            for p in (50, 95, 99, 100):
                row['wake_max_ms' if p == 100 else f'wake_p{p}_ms'] = q.get(f'p{p}_ms', '')
            writer.writerow(row)
    print(f'{len(records)} validated trials')


if __name__ == '__main__':
    main()
