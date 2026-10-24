"""Evidence parser regressions. Synthetic text is never native evidence."""
import unittest

from fastlane_f11b0c_validate import assess_native


def transcript():
    rows = ["[GRANT-C] client pid=62464 uid=1000 gid=1000 phase=invalid-issuer-and-proof PASS"]
    rows += [
        f"[GRANT-C] commit issuer=32 issuer_inc=9 role=uac recipient=62464 incarnation=1 vm=1 request={i} batch={1 if i == 5 else 2} ordinal={i} cpu=0"
        for i in range(1, 7)
    ]
    rows += ["[GRANT-C] owner-cancel recipient=62464 retired=1 inventory=0 cpu=0"] * 3
    rows += ["[GRANT-C] exec recipient=62464 incarnation=1 old_vm=1 new_vm=2 old_token=denied inventory=0 cpu=0"]
    rows += ["[GRANT-C] reaped recipient=62464 incarnation=1 exit=0 cpu=0"]
    rows += ["[GRANT-C] metadata vfs_active=0 vfs_capacity=4 vfs_entry_bytes=192 auth_active=0 auth_capacity=4 allocator_drift=UNMEASURED"]
    rows += ["[GRANT-C] COMPLETE PASS cpus=1 clients=1 committed=11 delivered=9 consumed=4 owner_retired=3 delivery_retired=2 exec_retired=2 inventory=0 pending=0 issuer_refs=0 recipient_refs=0 remote_race=NOT_FORCED"]
    return "\n".join(rows)


class EvidenceTests(unittest.TestCase):
    def test_complete_counts_preserve_remote_boundary(self):
        result = assess_native(transcript(), 1)
        self.assertEqual(result["classification"], "PASS")
        self.assertEqual(result["remote_lifecycle_race"], "NOT FORCED")
        self.assertEqual(result["probes"]["N5_actual_pid_reuse"], "NOT FORCED")

    def test_truncation_and_cpu_mismatch_fail(self):
        self.assertEqual(assess_native(transcript().split("[GRANT-C] COMPLETE")[0], 1)["classification"], "FAIL")
        self.assertEqual(assess_native(transcript(), 2)["classification"], "FAIL")

    def test_inventory_and_retirement_mismatch_fail(self):
        for source, replacement in [("committed=11", "committed=12"), ("pending=0", "pending=1"), ("retired=1", "retired=0")]:
            self.assertEqual(assess_native(transcript().replace(source, replacement), 1)["classification"], "FAIL")

    def test_invalid_identity_or_commit_cpu_fails(self):
        for source, replacement in [("issuer_inc=9", "issuer_inc=0"), ("vm=1 request", "vm=0 request"), ("batch=2", "batch=1"), ("cpu=0", "cpu=1")]:
            self.assertEqual(assess_native(transcript().replace(source, replacement), 1)["classification"], "FAIL")

    def test_failure_after_success_marker_still_fails(self):
        self.assertEqual(assess_native(transcript() + "\n[GRANT-C] client FAIL", 1)["classification"], "FAIL")


if __name__ == "__main__":
    unittest.main()
