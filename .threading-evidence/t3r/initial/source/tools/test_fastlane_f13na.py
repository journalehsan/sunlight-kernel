import unittest

from test_native_doorbell import ROUNDS, case_number, validate_native


class DoorbellEvidenceTests(unittest.TestCase):
    def parser_fixture(self, cpus=2):
        lines = [f'[F13NA] cpu cpu={cpu} apic={cpu} timer=masked' for cpu in range(cpus)]
        for round_id in range(ROUNDS):
            case = case_number(round_id)
            immediate = case in (1, 2, 3, 11)
            killed = case in (13, 14)
            registered = 0 if immediate else 2 if case == 9 else 1
            blocked = 0 if immediate or case in (4, 5, 13) else 2 if case == 9 else 1
            resumes = 0 if killed else 2 if case == 9 else 1
            wakes = 0 if immediate or killed else 2 if case == 9 else 1
            notifications = 8 * max(1, cpus - 1) if case == 8 else 0 if killed or case == 10 else 1
            source_mask = (1 if cpus == 1 or immediate or case == 6 else
                           ((1 << cpus) - 1) & ~1 if case == 8 else
                           1 << (1 + round_id % (cpus - 1)))
            classification = 'NOT_FORCED' if cpus == 1 and case in (4, 7, 13) else 'FORCED_AND_PASSED'
            outcome = ('work' if case in (1, 2) else 'changed' if case == 3 else
                       'consumer_exit' if killed else 'exhausted' if case == 15 else
                       'producer_exit' if round_id == 17 else 'closed' if case in (10, 11) else
                       'cancelled' if round_id == 11 else 'notified')
            sequence = (1 << 64) - 1 if case == 15 else notifications
            snapshot = (1 << 64) - 1 if case == 15 else 1 if case == 1 else 0
            lines.append(
                f'[F13NA] race round={round_id} case=N{case} result={classification} '
                f'generation={round_id + 1} incarnation={round_id + 1} snapshot={snapshot} sequence={sequence} '
                f'registered={registered} blocked={blocked} wakes={wakes} notifications={notifications} '
                f'sources={source_mask:#x} consumer=0 resumes={resumes} selections=1 '
                f'prepark={int(case in (4, 5, 13))} excluded={int(cpus > 1)} '
                f'stale_rejected={int(round_id > 0)} ready={wakes} spurious={int(case == 9)} outcome={outcome} cleanup=retired')
            lines.append(f'[F13NA] cleanup round={round_id} waiter=0 task_refs=0 queued=0 state=Reaped retired=1')
            lines.append(f'[F13NA] metrics round={round_id} sends=1 handlers=1 idle_ns={100000000 if case == 7 else 0} idle_handlers=0 ticks=0')
        lines += [f'[F13NA] workload cpu={cpu} user_executions=1' for cpu in range(cpus)]
        lines.append(f'[F13NA] PASS cpus={cpus} rounds=42 created=43 retired=43 active=0 waiters=0 task_refs=0 obligations=0 deferred=0 pending=0 queued=0 pins=0 frames_before=100 frames_after=100 metadata_heap_allocations=0 polling_wakes=0')
        return '\n'.join(lines)

    def test_parser_accepts_complete_fixture_contract(self):
        for cpus in (1, 2, 4, 12):
            row = validate_native(self.parser_fixture(cpus), cpus)
            self.assertEqual(row['classification'], 'FORCED_AND_PASSED', row['errors'])

    def test_boot_and_completion_alone_are_insufficient(self):
        self.assertNotEqual(validate_native('[SunlightOS] Phase 2.6 OK', 2)['classification'], 'FORCED_AND_PASSED')
        final_only = self.parser_fixture().splitlines()[-1]
        self.assertNotEqual(validate_native(final_only, 2)['classification'], 'FORCED_AND_PASSED')

    def test_missing_transition_execution_and_cleanup_are_rejected(self):
        for prefix in ('[F13NA] race round=5 ', '[F13NA] metrics round=5 ',
                       '[F13NA] cleanup round=5 ', '[F13NA] workload cpu=1 '):
            raw = '\n'.join(line for line in self.parser_fixture().splitlines() if not line.startswith(prefix))
            self.assertEqual(validate_native(raw, 2)['classification'], 'FORCED_AND_FAILED', prefix)

    def test_false_race_claims_are_rejected(self):
        for old, new in [('blocked=1', 'blocked=0'), ('resumes=1', 'resumes=0'),
                         ('excluded=1', 'excluded=0'), ('prepark=1', 'prepark=0'),
                         ('ready=1', 'ready=2'), ('stale_rejected=1', 'stale_rejected=0'),
                         ('outcome=closed', 'outcome=notified'), ('spurious=1', 'spurious=0'),
                         ('frames_after=100', 'frames_after=99'), ('deferred=0', 'deferred=1'),
                         ('snapshot=18446744073709551615', 'snapshot=0'),
                         ('generation=3 ', 'generation=1 '), ('sources=0x2', 'sources=0x1')]:
            self.assertEqual(validate_native(self.parser_fixture().replace(old, new), 2)['classification'],
                             'FORCED_AND_FAILED', old)

    def test_panic_and_unrelated_timer_assistance_are_visible(self):
        self.assertEqual(validate_native(self.parser_fixture() + '\n[KERNEL PANIC]', 2)['classification'], 'FORCED_AND_FAILED')
        raw = self.parser_fixture().replace('idle_handlers=0 ticks=0', 'idle_handlers=0 ticks=1')
        self.assertEqual(validate_native(raw, 2)['classification'], 'FORCED_AND_FAILED')


if __name__ == '__main__':
    unittest.main()
