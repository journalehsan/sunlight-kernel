"""Synthetic parser unit inputs are never execution evidence."""
import unittest

from fastlane_f11b0f_validate import assess_f


IDENTITY = dict.fromkeys(("source_unchanged", "iso_matches", "elf_matches", "embedded_match"), True)


def fixture():
    lines = ["[GRANT-F] pid-boundary max=18446744073709551615 last=accepted exhausted=None rollover=denied scope=isolated-real-allocator"]
    for index in range(8):
        lines += [f"[GRANT-F] pid-published parent=62704 pid={63000+index} incarnation={100+index} vm={200+index} roles=0 cpu=0 allocator=max-retained-plus-one",
                  f"[GRANT-F] pid-reaped pid={63000+index} incarnation={100+index} vm={200+index} pending=0 reply=0 deferred=0 stack=0 cpu=0"]
    for index, case in enumerate(("exit-consume", "exec-consume")):
        pid = 0xF4F3 + index
        lines.append(f"[GRANT-F] lifecycle case={case} checkpoint=commit pid={pid} incarnation=90 vm=99 issuer=20 issuer_inc=30 issuer_vm=40 request=5 fingerprint=boot-local-1 cpu=0")
        if index:
            lines.append(f"[GRANT-F] lifecycle case={case} checkpoint=exec pid={pid} incarnation=90 old_vm=99 new_vm=100 retired=2 replay=denied cpu=0")
        lines += [f"[GRANT-F] lifecycle case={case} checkpoint=reaped pid={pid} incarnation=90 exit={0 if index else 137} pending=0 reply=0 deferred=0 cpu=0",
                  f"[GRANT-F] lifecycle case={case} checkpoint=consume-attempt pid={pid} incarnation=90 old_vm=99 issuer=21 issuer_inc=31 issuer_vm=41 inventory=0 recipient=retiring-or-unpublished cpu=0",
                  f"[GRANT-F] consumer-result case={case} allowed=0"]
    lines.append("[GRANT-F] consumer-progress retired_exit=1 retired_exec=1 stale_reply=excluded PASS")
    for phase in ("start", "operation", "final"):
        lines.append(f"[GRANT-F] census phase={phase} broker_live=4 ipc_tokens=4 shm=0 pending=0 delivered=0 legacy=0 retiring_subset=0 sessions=0 receipts=0 pending_calls=0 reply_targets=0 deferred_subset=0 tombstones_subset=0 stored_replies=0 queued_calls=0 waiters=0 stale_queue_refs=0 ipc_capacity_bytes=0 history=4 history_capacity=256 snapshot=scheduler-broker-shards userspace_heaps=UNMEASURED global_reconciliation=INCOMPLETE cpu=0")
    lines.append("[GRANT-F] pid-contract cycles=8 same_numeric_pid=0 policy=non-reuse scope=ordinary-runtime-spawn PASS")
    start = next(line for line in lines if "census phase=start " in line)
    lines.remove(start)
    lines.insert(1, start)
    return "\n".join(lines)


class EvidenceParser(unittest.TestCase):
    def test_scoped_contract_is_not_numeric_reuse_or_global_reconciliation(self):
        result = assess_f(fixture(), 1, IDENTITY)
        self.assertEqual(result["errors"], [])
        self.assertEqual(result["forced_numeric_reuse"], "NOT_FORCED")
        self.assertEqual(result["global_reconciliation"], "BLOCKED")

    def test_requires_artifact_verification(self):
        self.assertTrue(assess_f(fixture(), 1)["errors"])
        self.assertTrue(assess_f(fixture(), 1, {**IDENTITY, "iso_matches": False})["errors"])

    def test_missing_duplicate_incomplete_and_invented_claims_rejected(self):
        raw = fixture()
        for corrupt in (raw.replace("pid=63000", "pid=63001"),
                        raw.replace("incarnation=100", "incarnation=0"),
                        raw.replace("global_reconciliation=INCOMPLETE", "global_reconciliation=PASS"),
                        raw.replace("stale_queue_refs=0", "stale_queue_refs=1"),
                        raw.replace("broker_live=4", "broker_live=5"),
                        raw.replace("cpu=0", "cpu=1"),
                        raw.replace("phase=final", "phase=unknown"),
                        raw.replace(" pending=0", " pending=1"),
                        raw.replace(" stack=0", " stack=1"),
                        raw.replace(" vm=200", ""),
                        raw.replace("new_vm=100", "new_vm=99"),
                        raw.replace("replay=denied", "replay=allowed"),
                        raw.replace("allowed=0", "allowed=1"),
                        raw.replace("checkpoint=consume-attempt", "checkpoint=unknown"),
                        raw.replace("new_vm=100", "new_vm=0"),
                        raw.replace("incarnation=90", "incarnation=0"),
                        raw + "\n[GRANT-F] lifecycle case=invented checkpoint=commit pid=1 incarnation=1 vm=1 issuer=2 issuer_inc=1 issuer_vm=1 request=1 cpu=0",
                        raw + "\n" + raw.splitlines()[1]):
            with self.subTest(corrupt=corrupt[-80:]):
                self.assertTrue(assess_f(corrupt, 1, IDENTITY)["errors"])

    def test_order_is_required(self):
        lines = fixture().splitlines()
        lines[2], lines[3] = lines[3], lines[2]
        self.assertTrue(assess_f("\n".join(lines), 1, IDENTITY)["errors"])
        lines = fixture().splitlines()
        start = next(line for line in lines if "census phase=start " in line)
        lines.remove(start)
        lines.append(start)
        self.assertTrue(assess_f("\n".join(lines), 1, IDENTITY)["errors"])


if __name__ == "__main__":
    unittest.main()
