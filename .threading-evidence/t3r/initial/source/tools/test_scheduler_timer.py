import unittest

from scheduler_timer import stages, validate_run


class TimerTraceTests(unittest.TestCase):
    def fixture(self):
        # CPU 0: publish, externally wake, select, receive, reply, caller resume.
        return dict(services=[dict(pid=2)], events=[
            [1, 100, 0, 0, 90, 10, 0], [2, 110, 0, 2, 10, 0, 0],
            [11, 115, 0, 2, 15, 6, 0], [3, 120, 0, 2, 1, 15, 6], [5, 130, 0, 2, 5, 7, 2],
            [4, 180, 0, 2, 6, 4, 15], [6, 190, 0, 2, 5, 7, 1000],
            [10, 200, 0, 2, 0, 0, 1008], [7, 210, 0, 2, 5, 7, 1010],
            [8, 300, 0, 5, 7, 3, 0],
        ])

    def test_end_to_end_decomposition_and_cpu_service_are_distinct(self):
        result = stages(self.fixture())
        s = result['series']
        self.assertEqual(s['irq_to_publication'], [20])
        self.assertEqual(s['publication_to_ready'], [5])
        self.assertEqual(s['wake_to_dispatch'], [65])
        self.assertEqual(s['ready_to_dispatch'], [60])
        self.assertEqual(s['request_to_receive'], [60])
        self.assertEqual(s['receive_to_reply'], [20])
        self.assertEqual(s['reply_service_cpu'], [10])
        self.assertEqual(s['reply_to_completion'], [90])
        self.assertEqual(s['request_to_completion'], [170])
        self.assertEqual(result['censored_requests'], 0)

    def test_missing_reply_cannot_be_a_completed_sample(self):
        run = self.fixture()
        run['events'] = [e for e in run['events'] if e[0] != 7]
        with self.assertRaisesRegex(ValueError, 'missing/duplicate IPC stage'):
            stages(run)

    def test_censored_wakes_and_requests_are_reported(self):
        run = self.fixture()
        run['events'] = run['events'][:5]
        result = stages(run)
        self.assertEqual(result['censored_requests'], 1)
        self.assertEqual(result['censored_ready'], 1)
        self.assertEqual(result['quantiles']['request_to_completion']['n'], 0)

    def test_coalesced_timer_sequence_is_endpoint_relative(self):
        run = dict(services=[dict(pid=2)], events=[
            [1, 100, 0, 0, 90, 100, 0], [2, 110, 0, 2, 100, 0, 0],
            [11, 115, 0, 2, 15, 6, 0],
            [2, 130, 0, 2, 101, 0, 0], [6, 150, 0, 2, 0, 12, 1000],
            [2, 160, 0, 2, 102, 0, 0], [6, 170, 0, 2, 0, 13, 1010],
        ])
        result = stages(run)
        self.assertEqual(result['tick_sequence_offset'], 89)
        self.assertEqual(result['series']['publication_to_tick_receive'], [40, 10])
        self.assertEqual(result['series']['latest_publication_to_tick_receive'], [20, 10])
        run['events'][-1][5] = 14
        with self.assertRaisesRegex(ValueError, 'association changed'):
            stages(run)

    def test_overflow_or_invalid_clock_is_rejected(self):
        for flag in ('overflow', 'invalid_clock'):
            run = dict(start=dict(run='1-1-1'), end=dict(run='1-1-1', status='ok', all_workers_progress=True),
                       trace=dict(run=1, count=0, overflow=False, invalid_clock=False), events=[])
            run['trace'][flag] = True
            with self.assertRaisesRegex(ValueError, 'overflow, invalid clock'):
                validate_run(run, False)

    def test_regressed_clock_and_wrong_cpu_are_rejected(self):
        for rows in ([[1, 100, 1, 0, 90, 10, 0]], [[1, 100, 0, 0, 90, 10, 0], [2, 99, 0, 2, 10, 0, 0]]):
            run = dict(start=dict(run='1-1-1'), end=dict(run='1-1-1', status='ok', all_workers_progress=True),
                       trace=dict(run=1, count=len(rows), overflow=False, invalid_clock=False), events=rows)
            with self.assertRaisesRegex(ValueError, 'CPU|regression'):
                validate_run(run, False)


if __name__ == '__main__':
    unittest.main()
