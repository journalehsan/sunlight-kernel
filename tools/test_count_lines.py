import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parents[1] / "count-lines.py"
SPEC = importlib.util.spec_from_file_location("count_lines", MODULE_PATH)
metrics = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(metrics)


class MetricsTests(unittest.TestCase):
    def test_physical_lines_and_unicode_binary(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "空间 file.rs").write_bytes("α\r\n\r\nlast".encode())
            (root / "bad.rs").write_bytes(b"\xff\xfe")
            (root / "binary.rs").write_bytes(b"a\0b\n")
            self.assertEqual(metrics.read_counts(root / "空间 file.rs"), (3, 2, False, False))
            self.assertEqual(metrics.read_counts(root / "bad.rs")[:3], (1, 1, False))
            self.assertTrue(metrics.read_counts(root / "binary.rs")[2])

    def test_git_inventory_and_generated_exclusions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "kernel").mkdir()
            (root / "vendor").mkdir()
            (root / "target").mkdir()
            (root / "kernel/a.rs").write_text("#[test]\nfn a() {}\n", encoding="utf-8")
            (root / "vendor/v.rs").write_text("vendor\n", encoding="utf-8")
            (root / "target/generated.rs").write_text("generated\n", encoding="utf-8")
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["kernel"]\n', encoding="utf-8")
            (root / "kernel/Cargo.toml").write_text("[package]\n", encoding="utf-8")
            import subprocess
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "add", "kernel", "Cargo.toml"], check=True)
            (root / "untracked.rs").write_text("untracked\n", encoding="utf-8")
            data = metrics.collect(root)
            self.assertEqual(data["tracked_files"], 3)
            self.assertEqual(data["rust"]["total"]["lines"], 2)
            self.assertEqual(data["rust"]["kernel"]["lines"], 2)
            self.assertEqual(data["cargo"]["manifest_count"], 2)
            self.assertEqual(data["cargo"]["workspace_member_count"], 1)
            self.assertEqual(data["tests_static_estimate"]["rust_files_with_test_indicators"], 1)

    def test_categories_subsystems_and_snapshots(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "kernel").mkdir()
            (root / "kernel/main.rs").write_text("one\n", encoding="utf-8")
            (root / "new-module").mkdir()
            (root / "new-module/lib.py").write_text("print(1)", encoding="utf-8")
            data = metrics.collect(root, "filesystem")
            self.assertEqual(data["source"]["lines"], 2)
            self.assertEqual(data["subsystems"]["Unclassified"]["files"], 1)
            self.assertEqual(sum(x["lines"] for x in data["subsystems"].values()), data["source"]["lines"])
            old = metrics.snapshot(data)
            newer = json.loads(json.dumps(old))
            newer["date"] = "later"
            newer["metrics"]["source_lines"] += 1000
            self.assertEqual(metrics.compare(old, newer)["changes"]["source_lines"]["absolute"], 1000)
            self.assertIn("1,000", metrics.human({**data, "source": {**data["source"], "lines": 1000}}))

    def test_empty_unterminated_docx_and_parallel_option_consistency(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "empty.py").write_bytes(b"")
            (root / "tail.py").write_bytes(b"one")
            (root / "report.docx").write_bytes(b"not parsed as text")
            sequential = metrics.collect(root, "filesystem", workers=0)
            configured = metrics.collect(root, "filesystem", workers=4)
            self.assertEqual(sequential["source"], configured["source"])
            self.assertEqual(sequential["source"]["lines"], 1)
            self.assertEqual(sequential["documentation"]["docx_files"], 1)
            self.assertIsNone(sequential["documentation"]["docx_line_count"])
            self.assertEqual(metrics.read_counts(root / "missing.rs")[:3], (0, 0, True))


if __name__ == "__main__":
    unittest.main()
