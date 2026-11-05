#!/usr/bin/env python3
"""Reject incomplete or incomparable fairness evidence, using a synthetic suite."""
import copy
import json
import unittest

from scheduler_fairness import validate_suite


def suite_records():
    records = []
    matrix = [(1000, 1, order) for order in ('forward', 'reverse')]
    matrix += [(duration, rep, order) for rep in range(1, 6)
               for duration in (5000, 10000) for order in ('forward', 'reverse')]
    for run, (duration, repetition, order) in enumerate(matrix, 1):
        identity = {'run': f'1-{run}-1'}
        records.append(('START', dict(identity, seed=1, cpus=1, workers=4, samples=1,
                                       duration_ms=duration, repetition=repetition, order=order)))
        for slot in range(4):
            records.append(('WORKER', dict(identity, slot=slot, pid=100 + (slot if order == 'forward' else 3 - slot), reaped=True, nice=0, mask=1,
                invalid_observations=0, wait_overflow=False, dispatches=1, wait_count=1,
                wait_ns=100, wait_max_ns=100, cpu_ns=duration * 1_000_000 // 4)))
            records.append(('WAIT', dict(identity, slot=slot, index=0, ns=100)))
            records.append(('LIFECYCLE', dict(identity, slot=slot, created_ms=0, ready_ms=1,
                start_ms=2, entered_ms=3, exited_ms=2 + duration)))
        records.append(('SAMPLE', dict(identity, index=0, elapsed_ms=duration, operations=400,
            ops_per_sec=400000 // duration, worker0=100, worker1=100, worker2=100, worker3=100)))
        records.append(('END', dict(identity, sample_count=1, status='ok', elapsed_ms=duration,
            operations=400, all_workers_progress=True)))
    return records


def raw(records):
    return '\n'.join(f'[SCHED-BENCH] {kind} {json.dumps(value)}' for kind, value in records) + \
        '\n[SCHED-BENCH] SUITE complete runs=22\n'


class SuiteTests(unittest.TestCase):
    def test_complete(self):
        result = validate_suite(raw(suite_records()))
        self.assertEqual(len(result['runs']), 22)
        self.assertEqual([v['runs'] for v in result['summary']], [1, 1, 5, 5, 5, 5])
        self.assertEqual(result['runs'][0]['cpu_fairness']['jain'], 1)

    def test_reject_bad_evidence(self):
        edits = [('WORKER', 'pid', 103), ('WORKER', 'nice', 1), ('WORKER', 'reaped', False),
                 ('WORKER', 'invalid_observations', 1), ('WORKER', 'wait_overflow', True),
                 ('WORKER', 'wait_ns', 101), ('LIFECYCLE', 'ready_ms', 4),
                 ('LIFECYCLE', 'exited_ms', 10), ('SAMPLE', 'operations', 401),
                 ('START', 'order', 'reverse'), ('WAIT', 'run', '1-99-1')]
        for kind, field, value in edits:
            with self.subTest(kind=kind, field=field):
                records = copy.deepcopy(suite_records())
                next(v for k, v in records if k == kind)[field] = value
                with self.assertRaises(ValueError):
                    validate_suite(raw(records))

    def test_missing_wait(self):
        records = suite_records()
        records.pop(next(i for i, (k, _) in enumerate(records) if k == 'WAIT'))
        with self.assertRaises(ValueError):
            validate_suite(raw(records))

    def test_missing_completion(self):
        with self.assertRaises(ValueError):
            validate_suite(raw(suite_records()).split('[SCHED-BENCH] SUITE')[0])


if __name__ == '__main__':
    unittest.main()
