"""Evidence integrity: reject incomplete windows and never cross generations."""
import unittest

from d38w_idle import parse, service_metrics


def serial(change_generation=False, regress=False):
    lines = ["[SunlightOS] sunlightd OK"]
    for i in range(1, 8):
        at = i * 5_000_000_000
        generation = 2 if change_generation and i >= 5 else 1
        count = 0 if regress and i == 5 else i * 100
        lines += [f"[CPU-AUDIT] core=0 timer_s=100 ipc_wake_s=20",
                  f"[WAKE-AUDIT] sample at_ns={at} dt_ns=5000000000 runnable=0 tasks=1 limit=64",
                  f"[WAKE-AUDIT] task at_ns={at} pid=12 gen={generation} name=sessiond state=2 wakes={count} ipc={count} recv_exp={count} call_exp=0 runtime_ns={i}",
                  f"[CPU-AUDIT] system at_ns={at} wake_s=20 cs_s=40 busy_bp=100"]
    return "\n".join(lines)


class EvidenceTests(unittest.TestCase):
    def test_warmup_and_exact_counter_delta(self):
        result = parse(serial())
        self.assertEqual(len(result["windows"]), 3)
        self.assertEqual(result["windows"][0]["system"]["at_ns"], 20_000_000_000)
        self.assertEqual(result["windows"][0]["tasks"][0]["wakes_s"], 20)

    def test_pid_reuse_does_not_cross_generation(self):
        result = parse(serial(change_generation=True))
        self.assertEqual(result["windows"][1]["tasks"], [])
        self.assertEqual(result["windows"][2]["tasks"][0]["generation"], 2)
        self.assertIsNone(service_metrics(result["windows"], "sessiond")["wakes_s"])

    def test_missing_ready_is_rejected(self):
        with self.assertRaises(ValueError):
            parse(serial().replace("[SunlightOS] sunlightd OK", ""))

    def test_short_capture_is_rejected(self):
        with self.assertRaises(ValueError):
            parse("\n".join(serial().splitlines()[:11]))

    def test_counter_regression_is_rejected(self):
        with self.assertRaises(ValueError):
            parse(serial(regress=True))

    def test_partial_new_report_is_not_a_completed_window(self):
        with self.assertRaisesRegex(ValueError, "still incomplete"):
            parse(serial().replace("limit=64", "limit=64 wakes_delta=100"))


if __name__ == "__main__":
    unittest.main()
