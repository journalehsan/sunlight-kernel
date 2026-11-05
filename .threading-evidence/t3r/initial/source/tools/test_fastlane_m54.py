"""Native-receipt rejection controls for the M5.4 measurement foundation."""
import os
import re
import unittest
from pathlib import Path

from fastlane_m54_receipt import assess

IDENTITY = dict(source=True, elf=True, iso=True, services=True)


def _synthetic(prefix_lines, cpus=2):
    """Build a minimal well-formed receipt, then let each test corrupt it."""
    lines = [
        "[FL-M54-CLOCK] status=ready source=pit-calibrated-tsc epoch=1 tsc_hz=2495000000 calibration_ppm=251 flags=0x14 invariant_tsc=0 contract=same-cpu-only cross_cpu=not-qualified",
        "[FL-M54-CLOCKSYNC] pid=40 cpu=0 accepted=1 stability_ppm=300 flags=36",
        "[FL-M54-CLOCKSYNC] pid=40 cpu=1 accepted=1 stability_ppm=300 flags=36",
    ]
    for _ in range(4):
        for pid, cpu in ((40, 0), (40, 1), (41, 0), (41, 1)):
            lines.append(
                "[FL-M54-CLOCK] pid=%d cpu=%d status=ready resident=1 reads=257 span_ns=900000 "
                "regressions=0 cpu_mismatch=0 epoch=3 unc_ppm=1300 unc_ns=2100 flags=36 "
                "stab_ok=1 contract=same-cpu-only" % (pid, cpu)
            )
        for pid in (40, 41):
            lines.append(
                "[FL-M54-XCPU] pid=%d pairs=3 rejected=3 min_delta_ns=1000 max_delta_ns=2000 "
                "contract=cross-cpu-subtraction-rejected" % pid
            )
            lines.append(
                "[FL-M54-REJECT] pid=%d backward=1 uncalibrated=1 overflow=1 cross_cpu=1 stale_epoch=1 "
                "time_backward=1 unsupported=0 all_rejected=1" % pid
            )
            lines.append(
                "[FL-M54-KCPU] generation=1 pid=%d charged_ns=1000 charged_valid=1 discontinuities=0 "
                "switches=4 wall_ns=5000 wall_valid=1 cpu_start=0 cpu_stop=0 migrated=0 reconcile=consistent "
                "wall_contract=same-cpu-only clock=tsc-qualified" % pid
            )
            lines.append(
                "[FL-M54-RES] pid=%d scope=whole-system total_kb=1 used_kb=2 swap_used_kb=0 swap_compressed_kb=0 "
                "attribution=scoped-global-delta physical_reclaim=not-proven interference=concurrent-services clock_hz_unit=1ns" % pid
            )
    return "\n".join(lines) + prefix_lines


class Structural(unittest.TestCase):
    def setUp(self):
        self.raw = _synthetic("")

    def rejected(self, raw):
        self.assertEqual(assess(raw, 2, IDENTITY)["status"], "FAIL")

    def test_clean_receipt_passes(self):
        self.assertEqual(assess(self.raw, 2, IDENTITY)["errors"], [])

    def test_missing_and_unsupported_input(self):
        self.assertTrue(assess("", 2, IDENTITY)["status"] == "FAIL")
        self.assertTrue(assess(self.raw, 3, IDENTITY)["status"] == "FAIL")
        self.assertTrue(assess(self.raw, 2, IDENTITY, measurement=False)["status"] == "FAIL")

    def test_identity_mismatch_rejected(self):
        self.assertEqual(assess(self.raw, 2, dict(IDENTITY, iso=False))["status"], "FAIL")

    def test_clock_not_ready_rejected(self):
        self.rejected(self.raw.replace("status=ready source=pit", "status=unsupported source=pit", 1))

    def test_duplicate_kernel_clock_rejected(self):
        first = self.raw.splitlines()[0]
        self.rejected(self.raw + "\n" + first)

    def test_regressing_probe_rejected(self):
        self.rejected(re.sub(r"regressions=0", "regressions=1", self.raw, count=1))

    def test_unbounded_probe_rejected(self):
        self.rejected(re.sub(r"unc_ppm=1300", "unc_ppm=0", self.raw, count=1))

    def test_probe_missing_contract_rejected(self):
        # 256-byte DebugLog truncation once silently dropped the contract
        # suffix; a probe without it must be rejected, never promoted.
        self.rejected(re.sub(r" stab_ok=1 contract=same-cpu-only", " stab_ok=1", self.raw, count=1))

    def test_probe_resident_status_mismatch_rejected(self):
        self.rejected(re.sub(r"status=ready resident=1", "status=ready resident=0", self.raw, count=1))

    def test_missing_probe_rejected(self):
        lines = self.raw.splitlines()
        i = next(i for i, l in enumerate(lines) if l.startswith("[FL-M54-CLOCK] pid="))
        self.rejected("\n".join(lines[:i] + lines[i + 1:]))

    def test_cross_cpu_silently_accepted_rejected(self):
        self.rejected(re.sub(r"rejected=3", "rejected=0", self.raw, count=1))

    def test_incomplete_rejection_self_check_rejected(self):
        self.rejected(re.sub(r"all_rejected=1", "all_rejected=0", self.raw, count=1))
        self.rejected(re.sub(r"backward=1", "backward=0", self.raw, count=1))

    def test_cpu_time_discontinuity_rejected(self):
        self.rejected(re.sub(r"discontinuities=0", "discontinuities=1", self.raw, count=1))

    def test_cpu_time_exceeding_wall_rejected(self):
        self.rejected(re.sub(r"charged_ns=1000 charged_valid=1", "charged_ns=999999 charged_valid=1", self.raw, count=1))

    def test_migration_with_cross_cpu_wall_rejected(self):
        self.rejected(re.sub(r"cpu_stop=0 migrated=0", "cpu_stop=1 migrated=1", self.raw, count=1))

    def test_resource_overclaim_rejected(self):
        self.rejected(re.sub(r"physical_reclaim=not-proven", "physical_reclaim=proven", self.raw, count=1))

    def test_corruption_of_every_numeric_field_rejected(self):
        for prefix, field in (
            ("[FL-M54-CLOCK] pid=", "span_ns"),
            ("[FL-M54-CLOCK] pid=", "regressions"),
            ("[FL-M54-XCPU]", "pairs"),
            ("[FL-M54-KCPU]", "charged_ns"),
        ):
            lines = self.raw.splitlines()
            i = next(i for i, l in enumerate(lines) if l.startswith(prefix))
            lines[i] = re.sub(r"\b" + field + r"=\d+", field + "=99999999999999999999", lines[i])
            self.rejected("\n".join(lines))


@unittest.skipUnless(os.environ.get("M54_SERIAL"), "M54_SERIAL required for retained guest controls")
class Native(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = Path(os.environ["M54_SERIAL"]).read_text(errors="replace")
        cls.cpus = int(os.environ.get("M54_CPUS", "2"))

    def test_original(self):
        self.assertEqual(assess(self.raw, self.cpus, IDENTITY)["errors"], [])


if __name__ == "__main__":
    unittest.main()
