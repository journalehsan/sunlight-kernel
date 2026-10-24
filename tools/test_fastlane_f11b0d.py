"""Negative evidence-parser tests. These strings are not native evidence."""
import unittest
from fastlane_f11b0d_validate import assess_races


def transcript():
    return "\n".join([
        "[GRANT-D] tables phase=start vfs_size=160 vfs_len=0 vfs_capacity=0",
        "[GRANT-D] accounting phase=start broker_live=3 ipc=1 shm=1 pending=1 delivered=0 legacy=0 history=3 history_capacity=256 history_bytes=24 history_capacity_bytes=2048",
        "[GRANT-D] case=exit-before-commit checkpoint=signal-before-exit issuer_driver=62704 driver_inc=11 recipient=62705 incarnation=12 vm=13 session_generation=none inventory=0 cpu=0",
        "[GRANT-D] case=exit-before-commit checkpoint=uac-resumed result=aborted",
        "[GRANT-D] case=exit-after-commit checkpoint=commit issuer=32 issuer_inc=9 recipient=62706 incarnation=14 vm=15 session_generation=none grant_batch=postcommit count=2 cpu=0",
        "[GRANT-D] case=exit-after-commit checkpoint=signal-before-exit issuer_driver=62704 driver_inc=11 recipient=62706 incarnation=14 vm=15 session_generation=none inventory=2 cpu=0",
        "[GRANT-D] case=exit-after-commit checkpoint=uac-resumed result=committed",
        "[GRANT-D] driver COMPLETE precommit=aborted postcommit=retired stale_reply=excluded",
        "[GRANT-D] checkpoint=reaped recipient=62704 incarnation=11 exit=0 cpu=0",
        "[GRANT-D] checkpoint=reaped recipient=62705 incarnation=12 exit=137 cpu=0",
        "[GRANT-D] checkpoint=reaped recipient=62706 incarnation=14 exit=137 cpu=0",
        "[GRANT-D] tables phase=final vfs_size=160 vfs_len=0 vfs_capacity=4",
        "[GRANT-D] accounting phase=final broker_live=1 ipc=1 shm=0 pending=0 delivered=0 legacy=0 history=7 history_capacity=256 history_bytes=56 history_capacity_bytes=2048",
        "[GRANT-D] lifecycle COMPLETE precommit_commits=0 postcommit_commits=2 inventory=0 reaped=3 pid_reuse=NOT_FORCED restart=NOT_FORCED",
    ])


class EvidenceTests(unittest.TestCase):
    def test_boundaries_are_preserved(self):
        result = assess_races(transcript(), 1)
        self.assertEqual(result["classification"], "FORCED_AND_PASSED")
        self.assertEqual(result["session_races"], "BLOCKED")
        self.assertEqual(result["pid_reuse"], "NOT_FORCED")

    def test_missing_order_identity_cpu_and_inventory_fail(self):
        for before, after in [("checkpoint=commit", "checkpoint=omitted"),
                              ("vm=15 session", "vm=0 session"),
                              ("inventory=2", "inventory=0"),
                              ("cpu=0", "cpu=1"),
                              ("history_bytes=56", "history_bytes=8"),
                              ("broker_live=1", "broker_live=2")]:
            self.assertEqual(assess_races(transcript().replace(before, after), 1)["classification"], "FORCED_AND_FAILED")

    def test_reaper_identity_must_match_the_participant(self):
        raw = transcript().replace("recipient=62706 incarnation=14 exit=137", "recipient=62706 incarnation=99 exit=137")
        self.assertTrue(assess_races(raw, 1)["errors"])

    def test_failure_after_complete_is_not_hidden(self):
        self.assertTrue(assess_races(transcript() + "\nPANIC", 1)["errors"])

    def test_duplicate_checkpoint_fails(self):
        self.assertTrue(assess_races(transcript() + "\n" + transcript(), 1)["errors"])


if __name__ == "__main__":
    unittest.main()
