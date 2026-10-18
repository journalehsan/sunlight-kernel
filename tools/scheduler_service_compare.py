#!/usr/bin/env python3
"""Compare complete C1.2B/C1.2C single-vCPU serial captures without dispatch proxies.

Writes validated per-worker CSV/JSON beside each input (via the fairness CLI),
and a comparison JSON with variability, throughput and exact Ready-wait quantiles.
Optional SERVICE observations must be complete and overflow-free when present.
"""
import argparse
import json
from pathlib import Path
import re
import statistics as st

from scheduler_fairness import percentile, validate_suite


def distribution(values):
    return dict(n=len(values), mean=st.mean(values), sd=st.pstdev(values),
                min=min(values), max=max(values))


def services(raw, runs):
    records = [json.loads(m.group(1)) for line in raw.splitlines()
               if (m := re.fullmatch(r'\[SCHED-BENCH\] SERVICE (\{.*\})', line.strip()))]
    if not records:
        return None
    expected = {(f'1-{r["run"]}-1', name) for r in runs
                for name in ('timer_server', 'tty_server')}
    if len(records) != len(expected) or {(r['run'], r['name']) for r in records} != expected:
        raise ValueError('incomplete or duplicate service records')
    if any(r['overflow'] or not r['waits_ns'] or
           any(not isinstance(ns, int) or ns < 0 for ns in r['waits_ns']) for r in records):
        raise ValueError('overflow or missing/invalid service waits')
    return records


def summarize(raw):
    suite = validate_suite(raw)
    records = services(raw, suite['runs'])
    groups = []
    for duration in (1000, 5000, 10000):
        for order in ('forward', 'reverse'):
            runs = [r for r in suite['runs'] if r['duration_ms'] == duration and r['order'] == order]
            waits = [w['ns'] for r in runs for worker in r['workers'] for w in worker['waits']]
            operations = [sum(w['operations'] for w in r['workers']) for r in runs]
            group = dict(duration_ms=duration, order=order,
                         cpu_jain=distribution([r['cpu_fairness']['jain'] for r in runs]),
                         work_jain=distribution([r['work_fairness']['jain'] for r in runs]),
                         cpu_max_min=distribution([r['cpu_fairness']['max_min'] for r in runs]),
                         operations=distribution(operations),
                         operations_per_second=distribution([ops * 1000 / r['elapsed_ms']
                             for ops, r in zip(operations, runs)]),
                         operations_per_cpu_second=distribution([ops * 1e9 / sum(w['cpu_ns'] for w in r['workers'])
                             for ops, r in zip(operations, runs)]),
                         dispatches=distribution([sum(w['dispatches'] for w in r['workers']) for r in runs]),
                         worker_wait_ms={f'p{p}': percentile(waits, p) / 1e6 for p in (50, 95, 99, 100)},
                         cpu_share=[distribution([r['workers'][slot]['cpu_ns'] /
                             sum(w['cpu_ns'] for w in r['workers']) for r in runs]) for slot in range(4)])
            if records:
                ids = {f'1-{r["run"]}-1' for r in runs}
                group['services'] = {}
                for name in ('timer_server', 'tty_server'):
                    ns = [ns for r in records if r['run'] in ids and r['name'] == name for ns in r['waits_ns']]
                    group['services'][name] = dict(n=len(ns), **{f'p{p}_ms': percentile(ns, p) / 1e6 for p in (50, 95, 99, 100)})
            groups.append(group)
    return dict(groups=groups, service_records=records)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = {label: summarize(path.read_text(errors='replace'))
              for label, path in [('before', args.before), ('after', args.after)]}
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print('| Seconds | Order | CPU Jain before → after (after min–max) | Work Jain before → after | ops/s before → after |')
    print('| ---: | --- | --- | --- | --- |')
    for before, after in zip(result['before']['groups'], result['after']['groups']):
        def mean(group, key): return group[key]['mean']
        print(f'| {after["duration_ms"] / 1000:g} | {after["order"]} | '
              f'{mean(before, "cpu_jain"):.4f} → {mean(after, "cpu_jain"):.4f} '
              f'({after["cpu_jain"]["min"]:.4f}–{after["cpu_jain"]["max"]:.4f}) | '
              f'{mean(before, "work_jain"):.4f} → {mean(after, "work_jain"):.4f} | '
              f'{mean(before, "operations_per_second"):.0f} → {mean(after, "operations_per_second"):.0f} |')


if __name__ == '__main__':
    main()
