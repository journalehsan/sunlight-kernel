"""Validate the added service latency evidence independently of worker framing."""
import copy
import unittest

from scheduler_service_compare import summarize
from test_scheduler_fairness import suite_records, raw as render


class ServiceEvidenceTests(unittest.TestCase):
    def records(self):
        records = suite_records()
        result = []
        for kind, value in records:
            if kind == 'END':
                for name in ('timer_server', 'tty_server'):
                    result.append(('SERVICE', dict(run=value['run'], name=name,
                                   overflow=False, waits_ns=[1, 100, 1000])))
            result.append((kind, value))
        return result

    def test_complete_latency_records(self):
        result = summarize(render(self.records()))
        self.assertEqual(len(result['service_records']), 44)
        self.assertEqual(result['groups'][2]['services']['timer_server']['n'], 15)

    def test_historical_capture_without_new_observer_is_allowed(self):
        self.assertIsNone(summarize(render(suite_records()))['service_records'])

    def test_missing_duplicate_overflow_and_invalid_waits_fail_closed(self):
        records = self.records()
        service = next(i for i, (kind, _) in enumerate(records) if kind == 'SERVICE')
        cases = []
        cases.append(records[:service] + records[service + 1:])
        cases.append(records + [records[service]])
        for change in [dict(overflow=True), dict(waits_ns=[]), dict(waits_ns=[-1])]:
            bad = copy.deepcopy(records)
            bad[service][1].update(change)
            cases.append(bad)
        for case in cases:
            with self.subTest(case=case[service]), self.assertRaises(ValueError):
                summarize(render(case))


if __name__ == '__main__':
    unittest.main()
