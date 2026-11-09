"""Synthetic rejection controls; these do not constitute native evidence."""
import unittest
from fastlane_m53r_assess import assess


def control():
    return "\n".join([
        "[FL-COORD] event=wait yields=1 checked_resumes=1 rsp_before=65000 rsp_after=65000 stack_top=65536 pid=1 inc=2 vm=3 resume_if=0",
        "[FL-FRAME] event=backing frame=8192",
        "[FL-QUOTA] event=reclaimed pid=1 inc=2 session=3 epoch=4 bytes=4096 frames=1 frame=8192 free_before=100 free_after=101 process_before=4096 process_after=0 session_before=4096 session_after=0 cpu=0",
        "[FL-QUOTA] event=checkpoint checkpoint=999 account_bytes=0 account_entries=0 restricted_receipts=0 objects=0 views=0 frames=0",
    ])


class M53RAssessorTests(unittest.TestCase):
    def test_positive_control(self):
        self.assertEqual(assess(control())["status"], "PASS")

    def test_empty_and_truncated(self):
        for raw in ("", "boot OK", control().split("[FL-QUOTA] event=checkpoint")[0]):
            self.assertEqual(assess(raw)["status"], "FAIL")

    def test_continuation_mismatches(self):
        for old, new in (("rsp_after=65000", "rsp_after=64000"),
                         ("checked_resumes=1", "checked_resumes=0"),
                         ("resume_if=0", "resume_if=1"), ("inc=2", "inc=0")):
            self.assertEqual(assess(control().replace(old, new))["status"], "FAIL")

    def test_quota_and_frame_mismatches(self):
        for old, new in (("process_after=0", "process_after=4096"),
                         ("session_after=0", "session_after=4096"),
                         ("free_after=101", "free_after=100"),
                         ("account_bytes=0", "account_bytes=4096"),
                         ("restricted_receipts=0", "restricted_receipts=1"),
                         ("frames=1 frame=8192", "frames=1 frame=12288")):
            self.assertEqual(assess(control().replace(old, new))["status"], "FAIL")

    def test_missing_or_duplicate_receipt(self):
        lines = control().splitlines()
        for raw in ("\n".join(lines[:2] + lines[3:]), control() + "\n" + lines[2],
                    control() + "\n" + lines[-1], control().replace("inc=2", "inc=2 inc=2")):
            self.assertEqual(assess(raw)["status"], "FAIL")

    def test_panic_after_complete(self):
        self.assertEqual(assess(control() + "\n[KERNEL PANIC]")["status"], "FAIL")


if __name__ == "__main__":
    unittest.main()
