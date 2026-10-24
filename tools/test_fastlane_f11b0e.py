"""Synthetic parser negatives are never native authority evidence."""
import unittest
from fastlane_f11b0e_validate import assess_authority_fallback


class ClassificationTests(unittest.TestCase):
    def test_ordinary_smoke_cannot_certify_session_or_capacity(self):
        result = assess_authority_fallback("Phase 1 OK\n", 12)
        self.assertEqual(result["N8"], "FORCED_AND_FAILED")
        self.assertEqual(set(result["native_session_cases"].values()), {"BLOCKED"})
        self.assertEqual(result["session_authority"], "BLOCKED")

    def test_historical_success_cannot_certify_disabled_account_path(self):
        result = assess_authority_fallback("[CAP-PR] lifecycle PASS auth=17 account=17 post_reap_records=0", 1)
        self.assertTrue(result["errors"])
        self.assertEqual(result["global_broker_exhaustion_and_error_delivery"], "NOT_RUN")
