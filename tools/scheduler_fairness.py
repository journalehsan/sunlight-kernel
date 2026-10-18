#!/usr/bin/env python3
"""Validate and summarize C1.2B measurements, reusing the C1.2A validator."""
import csv
import json
import math
from pathlib import Path
import re
import statistics
import sys

from scheduler_bench import validate_guest_report

RECORD = re.compile(r'^\[SCHED-BENCH\] (START|SAMPLE|END|WORKER|WAIT|LIFECYCLE) (\{.*\})$')


def fairness(values):
    mean = statistics.mean(values)
    return {'jain': sum(values) ** 2 / (len(values) * sum(x * x for x in values)),
            'cv': statistics.pstdev(values) / mean,
            'max_min': max(values) / min(values)}


def percentile(values, percent):
    return sorted(values)[max(0, math.ceil(len(values) * percent / 100) - 1)]


def validate_suite(raw):
    groups = []
    current = None
    for line in raw.splitlines():
        match = RECORD.match(line.strip())
        if not match:
            continue
        kind, body = match.groups()
        value = json.loads(body)
        if kind == 'START':
            if current is not None:
                raise ValueError('nested START')
            current = {'lines': [], 'records': []}
        if current is None:
            raise ValueError('record outside frame')
        current['lines'].append(line)
        current['records'].append((kind, value))
        if kind == 'END':
            groups.append(current)
            current = None
    if current is not None or len(groups) != 22 or '[SCHED-BENCH] SUITE complete runs=22' not in raw:
        raise ValueError('incomplete suite')
    results = []
    matrix = [(1000, 1, order) for order in ('forward', 'reverse')]
    matrix += [(duration, repetition, order) for repetition in range(1, 6)
               for duration in (5000, 10000) for order in ('forward', 'reverse')]
    for ordinal, (group, expected) in enumerate(zip(groups, matrix), 1):
        valid, reason = validate_guest_report('\n'.join(group['lines']), 1, 1, ordinal)
        if not valid:
            raise ValueError(f'run {ordinal}: {reason}')
        records = group['records']
        def values(kind):
            return [v for k, v in records if k == kind]
        start, end = values('START')[0], values('END')[0]
        if (start['duration_ms'], start['repetition'], start['order']) != expected:
            raise ValueError('matrix mismatch')
        sample = values('SAMPLE')[0]
        workers = sorted(values('WORKER'), key=lambda w: w['slot'])
        life = sorted(values('LIFECYCLE'), key=lambda w: w['slot'])
        if [w['slot'] for w in workers] != list(range(4)) or [w['slot'] for w in life] != list(range(4)):
            raise ValueError('incomplete workers/lifecycle')
        expected_creation = list(range(4)) if expected[2] == 'forward' else list(reversed(range(4)))
        if len({w['pid'] for w in workers}) != 4 or [w['slot'] for w in sorted(workers, key=lambda w: w['pid'])] != expected_creation:
            raise ValueError('creation order mismatch')
        if len({w['mask'] for w in workers}) != 1:
            raise ValueError('unequal affinity masks')
        run_id = start['run']
        if any(v.get('run') != run_id for _, v in records):
            raise ValueError('diagnostic run mismatch')
        counts = [sample[f'worker{i}'] for i in range(4)]
        if min(counts) <= 0 or sum(counts) != sample['operations'] or end['operations'] != sum(counts):
            raise ValueError('invalid operation counts')
        if sample['elapsed_ms'] < expected[0] or not end['all_workers_progress']:
            raise ValueError('incomplete measurement interval')
        if sample['elapsed_ms'] != end['elapsed_ms'] or sample['ops_per_sec'] != sum(counts) * 1000 // sample['elapsed_ms']:
            raise ValueError('inconsistent elapsed/throughput')
        for slot, (w, lifecycle) in enumerate(zip(workers, life)):
            if not w['reaped'] or w['nice'] != 0 or w['mask'] & 1 != 1 or w['invalid_observations'] or w['wait_overflow']:
                raise ValueError('ineligible/unfinished worker or overflow')
            if not lifecycle['created_ms'] <= lifecycle['ready_ms'] <= lifecycle['start_ms'] <= lifecycle['entered_ms'] <= lifecycle['exited_ms']:
                raise ValueError('invalid lifecycle order/barrier')
            if lifecycle['exited_ms'] < lifecycle['start_ms'] + expected[0]:
                raise ValueError('early worker completion')
            waits = [v for v in values('WAIT') if v['slot'] == slot]
            if [v['index'] for v in waits] != list(range(w['dispatches'])) or len(waits) != w['wait_count'] or not waits:
                raise ValueError('incomplete dispatch/wait evidence')
            times = [v['ns'] for v in waits]
            if sum(times) != w['wait_ns'] or max(times) != w['wait_max_ns']:
                raise ValueError('wait aggregation mismatch')
            w['wait_p50_ns'] = percentile(times, 50)
            w['wait_p95_ns'] = percentile(times, 95)
            w['wait_p99_ns'] = percentile(times, 99)
            w['operations'] = counts[slot]
            w['ops_per_cpu_second'] = counts[slot] * 1e9 / w['cpu_ns']
            w['entry_lag_ms'] = lifecycle['entered_ms'] - lifecycle['start_ms']
            w['exit_lag_ms'] = lifecycle['exited_ms'] - lifecycle['start_ms'] - expected[0]
            w['waits'] = waits
            w['lifecycle'] = lifecycle
        results.append({'run': ordinal, 'duration_ms': expected[0], 'repetition': expected[1],
                        'order': expected[2], 'elapsed_ms': sample['elapsed_ms'],
                        'workers': workers, 'work_fairness': fairness(counts),
                        'cpu_fairness': fairness([w['cpu_ns'] for w in workers]),
                        'dispatch_fairness': fairness([w['dispatches'] for w in workers])})
    summaries = []
    for duration in (1000, 5000, 10000):
        for order in ('forward', 'reverse'):
            subset = [r for r in results if r['duration_ms'] == duration and r['order'] == order]
            summaries.append({'duration_ms': duration, 'order': order, 'runs': len(subset),
                              'work_jain_mean': statistics.mean(r['work_fairness']['jain'] for r in subset),
                              'work_jain_min': min(r['work_fairness']['jain'] for r in subset),
                              'work_max_min_mean': statistics.mean(r['work_fairness']['max_min'] for r in subset),
                              'cpu_jain_mean': statistics.mean(r['cpu_fairness']['jain'] for r in subset),
                              'slot_work_share_mean': [statistics.mean(r['workers'][s]['operations'] /
                                  sum(w['operations'] for w in r['workers']) for r in subset) for s in range(4)],
                              'slot_work_share_sd': [statistics.pstdev(r['workers'][s]['operations'] /
                                  sum(w['operations'] for w in r['workers']) for r in subset) for s in range(4)]})
    return {'runs': results, 'summary': summaries}


if __name__ == '__main__':
    source = Path(sys.argv[1])
    result = validate_suite(source.read_text(errors='replace'))
    output = source.with_suffix('.fairness.json')
    output.write_text(json.dumps(result, indent=2) + '\n')
    csv_path = source.with_suffix('.workers.csv')
    columns = ['run', 'duration_ms', 'repetition', 'order', 'slot', 'pid', 'operations',
               'cpu_ns', 'dispatches', 'wait_ns', 'wait_count', 'wait_p50_ns',
               'wait_p95_ns', 'wait_p99_ns', 'wait_max_ns', 'entry_lag_ms',
               'exit_lag_ms', 'burst_start', 'burst_end', 'nice', 'mask',
               'invalid_observations', 'reaped', 'ops_per_cpu_second']
    with csv_path.open('w') as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, extrasaction='ignore', lineterminator='\n')
        writer.writeheader()
        for run in result['runs']:
            for worker in run['workers']:
                writer.writerow(dict(run, **worker))
    print(json.dumps(result['summary'], indent=2))
    print(f'Validated {len(result["runs"])} guest runs: {output}')
