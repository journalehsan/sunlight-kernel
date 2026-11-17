#!/usr/bin/env python3
"""Regression checks for the migration source guard (not a Rust parser)."""
import tempfile
import unittest
from pathlib import Path

from audit_vfs_cutover import additions, inventory


class MigrationGuardTests(unittest.TestCase):
    def test_removals_and_line_shifts_pass_but_replacements_do_not(self):
        row = {"path": "kernel/src/main.rs", "category": "kernel-global namespace", "code": "KERNEL_VFS.lock();"}
        self.assertFalse(additions([], [row]))
        self.assertFalse(additions([(row["path"], 999, row["category"], row["code"])], [row]))
        for field in ["path", "category", "code"]:
            changed = dict(row)
            changed[field] += "new"
            self.assertTrue(additions([(changed["path"], 1, changed["category"], changed["code"])], [row]))
        duplicate = (row["path"], 2, row["category"], row["code"])
        self.assertEqual(sum(additions([duplicate, duplicate], [row]).values()), 1)

    def test_inventory_scans_only_kernel_rust_and_ignores_line_comments(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "kernel/src").mkdir(parents=True)
            (root / "services").mkdir()
            (root / "services/main.rs").write_text("KERNEL_VFS.lock();\n")
            (root / "kernel/src/ignore.txt").write_text("KERNEL_VFS.lock();\n")
            (root / "kernel/src/main.rs").write_text(
                "// KERNEL_VFS.lock();\nlet fs = Vfs::new();\n"
                "let fs = Tmpfs::new(&[]);\nKERNEL_VFS.lock(); // existing owner\n"
            )
            rows = inventory(root)
            self.assertEqual(len(rows), 3)
            self.assertEqual([row[1] for row in rows], [2, 3, 4])
            self.assertEqual(rows[-1][3], "KERNEL_VFS.lock();")


if __name__ == "__main__":
    unittest.main()
