"""Synthetic parser rejection controls, never native proof."""
import unittest
from fastlane_m3_validate import assess_words, FIRST, SECOND
from fastlane_m3_regressions import manifest_passed


def trace():
    lines = []
    for generation in (1, 2):
        for region in range(3):
            for actor, pid, write in (("writer", 10, 1), ("reader", 11, 0)):
                lines.append(f"[FL-MAP] event=map generation={generation} region={region} pid={pid} va={pid * 4096} frame={generation * 4096 + region * 4096} write={write}")
            lines.append(f"[FL-M3-WORD] generation={generation} region={region} writer=10 reader=11 writer_va=40960 reader_va=45056 offset=4088 first={FIRST} second={SECOND} private_copy=stable")
    return "\n".join(lines)


class WordControls(unittest.TestCase):
    def test_control_is_word_only(self):
        self.assertEqual(assess_words(trace())["status"], "PASS")

    def test_empty(self):
        self.assertEqual(assess_words("")["status"], "FAIL")

    def test_missing(self):
        self.assertEqual(assess_words("\n".join(trace().splitlines()[:-1]))["status"], "FAIL")

    def test_permission_escalation(self):
        self.assertEqual(assess_words(trace().replace("write=0", "write=1"))["status"], "FAIL")

    def test_wrong_page_offset(self):
        self.assertEqual(assess_words(trace().replace("offset=4088", "offset=4096"))["status"], "FAIL")

    def test_wrong_va(self):
        self.assertEqual(assess_words(trace().replace("reader_va=45056", "reader_va=45057"))["status"], "FAIL")

    def test_corrupt_copy(self):
        self.assertEqual(assess_words(trace().replace("private_copy=stable", "private_copy=changed"))["status"], "FAIL")

    def test_duplicate_field(self):
        self.assertEqual(assess_words(trace().replace("offset=4088", "offset=4088 offset=4088"))["status"], "FAIL")

    def test_malformed(self):
        self.assertEqual(assess_words(trace().replace("offset=4088", "offset=bad"))["status"], "FAIL")


class ManifestControls(unittest.TestCase):
    def legacy(self):
        return {"images": {"fixture": {}}, "checks": [{"exit_code": 0}],
                "native": [{"validator": {"classification": "FORCED_AND_PASSED", "terminal_protocol": "PASS",
                                           "host_terminal_acknowledged": True, "host_ack": "original.json"}}]}

    def test_legacy_build_and_native(self):
        self.assertTrue(manifest_passed(self.legacy(), "build"))
        self.assertTrue(manifest_passed(self.legacy(), "native"))

    def test_legacy_failed_check(self):
        data = self.legacy(); data["checks"][0]["exit_code"] = 1
        self.assertFalse(manifest_passed(data, "build"))

    def test_legacy_missing_ack(self):
        data = self.legacy(); data["native"][0]["validator"]["host_terminal_acknowledged"] = False
        self.assertFalse(manifest_passed(data, "native"))

    def test_legacy_unforced(self):
        data = self.legacy(); data["native"][0]["validator"]["classification"] = "NOT_FORCED"
        self.assertFalse(manifest_passed(data, "native"))

    def test_legacy_missing_image(self):
        data = self.legacy(); data["images"] = {}
        self.assertFalse(manifest_passed(data, "build"))

    def test_current_schema_requires_pass_and_unchanged_source(self):
        self.assertTrue(manifest_passed({"status": "PASS"}, "native"))
        self.assertFalse(manifest_passed({"status": "RUNNING"}, "native"))
        self.assertFalse(manifest_passed({"status": "FAIL"}, "native"))
        self.assertFalse(manifest_passed({"status": "PASS", "source_hashes_unchanged": False}, "native"))


if __name__ == "__main__":
    unittest.main()
