#!/usr/bin/env python3
"""Host contract tests for scheduler guest report framing."""
import importlib.util
import json
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    'scheduler_bench', Path(__file__).with_name('scheduler_bench.py'))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def report(samples=2):
    run = '2-1-41'
    records = [('START', {'run': run, 'seed': 41, 'cpus': 2, 'samples': samples})]
    records += [('SAMPLE', {'run': run, 'index': i}) for i in range(samples)]
    records += [('END', {'run': run, 'sample_count': samples, 'status': 'ok'})]
    return '\n'.join(f'[SCHED-BENCH] {kind} {json.dumps(value)}'
                     for kind, value in records)


class ReportValidationTests(unittest.TestCase):
    def test_complete_report(self):
        self.assertEqual(MODULE.validate_guest_report(report(), 41, 2, 1), (True, None))

    def test_missing_end(self):
        raw = '\n'.join(report().splitlines()[:-1])
        self.assertEqual(MODULE.validate_guest_report(raw, 41, 2, 1)[1],
                         'missing_or_duplicate_boundary')

    def test_missing_sample(self):
        raw = '\n'.join(line for line in report().splitlines()
                        if '"index": 1' not in line)
        self.assertEqual(MODULE.validate_guest_report(raw, 41, 2, 1)[1],
                         'incomplete_samples')

    def test_duplicate_index(self):
        raw = report().replace('"index": 1', '"index": 0')
        self.assertEqual(MODULE.validate_guest_report(raw, 41, 2, 1)[1],
                         'sample_index_mismatch')

    def test_record_outside_boundaries(self):
        lines = report().splitlines()
        lines[0], lines[1] = lines[1], lines[0]
        self.assertEqual(MODULE.validate_guest_report('\n'.join(lines), 41, 2, 1)[1],
                         'record_order_mismatch')

    def test_configuration_mismatch(self):
        self.assertEqual(MODULE.validate_guest_report(report(), 42, 2, 1)[1],
                         'run_id_mismatch')


if __name__ == '__main__':
    unittest.main()
