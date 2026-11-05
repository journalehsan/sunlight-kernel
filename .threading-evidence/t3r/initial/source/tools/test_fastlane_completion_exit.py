"""Fail-closed mutation tests. Recorded excerpts are parser controls only."""
import re
import unittest
from fastlane_completion_exit_controls import CONTROLS
from fastlane_completion_exit_validate import assess_cycles

IDENTITY = dict(source_matches=True, services_match=True, iso_matches=True, elf_matches=True)


class ActualExitEvidenceTests(unittest.TestCase):
    def assess(self, raw, mode, fixture, **kw):
        return assess_cycles(raw, 2, [fixture], mode=mode, identity=kw.pop('identity', IDENTITY), **kw)

    def reject(self, raw, mode, fixture, **kw):
        self.assertEqual(self.assess(raw, mode, fixture, **kw)['status'], 'FAIL')

    def test_both_recorded_orders(self):
        for mode, fixture, raw in CONTROLS:
            result = self.assess(raw, mode, fixture)
            self.assertEqual(result['status'], 'PASS', result['errors'])

    def test_every_actual_exit_and_race_event_required(self):
        for mode, fixture, raw in CONTROLS:
            lines = raw.splitlines()
            for i, line in enumerate(lines):
                if line.startswith(('[EX-RACE]', '[EX-OUTCOME]')):
                    with self.subTest(mode=mode, line=line):
                        self.reject('\n'.join(lines[:i]+lines[i+1:]), mode, fixture)

    def test_same_cpu_or_late_entry_rejected(self):
        for mode, fixture, raw in CONTROLS:
            entry = next(l for l in raw.splitlines() if 'event=entered actor=2 ' in l)
            self.reject(raw.replace(entry, re.sub(r'cpu=\d+', 'cpu=1', entry)), mode, fixture)
            self.reject(raw.replace(entry, re.sub(r'seq=\d+', 'seq=999', entry)), mode, fixture)

    def test_identity_and_authority_mismatches_rejected(self):
        for mode, fixture, raw in CONTROLS:
            entry = next(l for l in raw.splitlines() if 'event=entered actor=1 ' in l)
            for key in ('pid', 'inc', 'caller', 'caller_inc', 'endpoint', 'generation'):
                self.reject(raw.replace(entry, re.sub(r'\b'+key+r'=\d+', key+'=999', entry)), mode, fixture)

    def test_duplicate_outcome_retirement_and_free_rejected(self):
        for mode, fixture, raw in CONTROLS:
            for prefix in ('[EX-RACE]', '[EX-OUTCOME]', '[AE-PMM]', '[CE-REAP]'):
                line = next(l for l in raw.splitlines() if l.startswith(prefix))
                self.reject(raw+'\n'+line, mode, fixture)
            line = next(l for l in raw.splitlines() if l.startswith('[AE-GRANT]') and 'event=retired ' in l)
            self.reject(raw+'\n'+line, mode, fixture)

    def test_reply_into_retired_caller_rejected(self):
        mode, fixture, raw = CONTROLS[1]
        self.reject(raw.replace('outcome=None state=Finished', 'outcome=Some(ReplyDelivered(5)) state=Finished'), mode, fixture)
        line = next(l for l in CONTROLS[0][2].splitlines() if l.startswith('[AE-TERMINAL]'))
        self.reject(raw+'\n'+line, mode, fixture)

    def test_valid_completion_must_survive_until_exit(self):
        mode, fixture, raw = CONTROLS[0]
        line = next(l for l in raw.splitlines() if l.startswith('[EX-OUTCOME]') and 'actor=1 ' in l)
        self.reject(raw.replace(line, line.replace('grants=1', 'grants=0')), mode, fixture)
        self.reject(raw.replace(line, line.replace('reply=1', 'reply=0')), mode, fixture)

    def test_no_synthetic_death_or_cancellation_substitute(self):
        for mode, fixture, raw in CONTROLS:
            self.reject(raw.replace('event=exit-syscall', 'event=signal-posted'), mode, fixture)
            self.reject(raw.replace('reason=exit code=0', 'reason=killed code=137'), mode, fixture)
            self.reject(raw.replace('event=exit-marked', 'event=cancelled'), mode, fixture)

    def test_original_mapping_and_quiescent_reaping_required(self):
        for mode, fixture, raw in CONTROLS:
            for prefix in ('[AE-UNMAP]', '[AE-TLB]', '[AE-PMM]', '[CE-REAP]'):
                self.reject('\n'.join(l for l in raw.splitlines() if not l.startswith(prefix)), mode, fixture)
            self.reject(raw.replace('active=0', 'active=1'), mode, fixture)
            self.reject(raw.replace('receipts=0', 'receipts=1'), mode, fixture)

    def test_late_checks_and_independent_parent_required(self):
        for mode, fixture, raw in CONTROLS:
            for event in ('calling-real-exit', 'late-completions-rejected', 'caller-endpoint-retired', 'foreign-adoption-denied', 'supervisor-clean'):
                self.reject('\n'.join(l for l in raw.splitlines() if 'event='+event+' ' not in l), mode, fixture)

    def test_failure_identity_and_guest_fault_rejected(self):
        for mode, fixture, raw in CONTROLS:
            self.reject(raw, mode, fixture, identity=None)
            self.reject(raw, mode, fixture, failure={'type': 'TimeoutError'})
            self.reject(raw+'\nPANIC', mode, fixture)


if __name__ == '__main__':
    unittest.main()
