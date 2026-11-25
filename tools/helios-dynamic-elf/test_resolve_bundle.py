import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from resolve_bundle import resolve


class BundleTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.bundle = Path(scratch.name)
        self.program = self.bundle / "Program"
        self.lib = self.program / "lib"
        self.lib.mkdir(parents=True)
        (self.bundle / "Manifest.toml").write_text(
            '[bundle]\nformat = 1\n[runtime]\ntype = "helios"\n'
            '[application]\nkind = "gui"\nid = "test.elf"\n[entry]\nexecutable = "Program/app"\n'
        )
        self.compile("void loader(void) {}", self.lib / "ld.so", "-shared", "-Wl,-soname,ld.so")
        self.version = self.bundle / "version"
        self.version.write_text("LEAF_1 { global: leaf; local: *; };")
        self.leaf()
        (self.lib / "libLeaf.so").symlink_to("libLeaf.so.1")
        self.compile("extern int leaf(void); void _start(void) { leaf(); }",
                     self.program / "app", "-pie", "-Wl,--dynamic-linker,/lib/ld.so",
                     "-Wl,-rpath,$ORIGIN/lib", f"-L{self.lib}", "-lLeaf")

    def compile(self, code, output, *flags):
        source = self.bundle / "unit.c"
        source.write_text(code)
        subprocess.run(["cc", "-nostdlib", "-fPIC", str(source), "-o", str(output), *flags],
                       check=True, capture_output=True)

    def leaf(self, *flags):
        self.compile("int leaf(void) { return 1; }", self.lib / "libLeaf.so.1",
                     "-shared", "-Wl,-soname,libLeaf.so", f"-Wl,--version-script={self.version}", *flags)

    def graph(self, plugins=(), directories=()):
        return resolve(self.bundle, "/Applications/Test.sunapp", directories, plugins,
                       {"/lib/ld.so": "Program/lib/ld.so"})

    def test_origin_versions_aliases_and_determinism(self):
        graph = self.graph()
        self.assertEqual("PASS", graph["status"])
        self.assertEqual(2, graph["resolved_dependencies"])
        self.assertEqual({"Program/lib/libLeaf.so": "libLeaf.so.1"}, graph["symlinks"])
        app = graph["objects"]["Program/app"]
        self.assertEqual({"libLeaf.so": ["LEAF_1"]}, app["version_requirements"])
        self.assertEqual(json.dumps(graph), json.dumps(self.graph()))

    def test_unresolved_local_dependency(self):
        (self.lib / "libLeaf.so").unlink()
        self.assertEqual(["libLeaf.so"], [edge["name"] for edge in self.graph()["unresolved"]])

    def test_incompatible_provider_version(self):
        self.version.write_text("LEAF_2 { global: leaf; local: *; };")
        self.leaf()
        self.assertIn("lacks versions LEAF_1", self.graph()["errors"][0])

    def test_runtime_plugin_widens_closure(self):
        plugin = self.program / "plugin.so"
        self.compile("extern int leaf(void); int plugin(void) { return leaf(); }",
                     plugin, "-shared", f"-L{self.lib}", "-lLeaf")
        self.assertEqual("PASS", self.graph()["status"])
        self.assertEqual("FAIL", self.graph(["Program/plugin.so"])["status"])
        self.assertEqual("PASS", self.graph(["Program/plugin.so"], ["Program/lib"])["status"])

    def test_conflicting_sonames(self):
        plugin = self.program / "plugin.so"
        self.compile("int other(void) { return 2; }", plugin, "-shared", "-Wl,-soname,libLeaf.so")
        self.assertIn("conflicting SONAME", self.graph(["Program/plugin.so"])["errors"][0])

    def test_symlink_escape(self):
        (self.lib / "libLeaf.so").unlink()
        (self.lib / "libLeaf.so").symlink_to("../../../elsewhere")
        self.assertIn("symlink escapes bundle", self.graph()["errors"][0])

    def test_cycle(self):
        self.compile("int branch(void) { return 2; }", self.lib / "libBranch.so",
                     "-shared", "-Wl,-soname,libBranch.so")
        self.leaf("-Wl,--no-as-needed", f"-L{self.lib}", "-lBranch")
        self.compile("extern int leaf(void); int branch(void) { return leaf(); }",
                     self.lib / "libBranch.so", "-shared", "-Wl,-soname,libBranch.so",
                     f"-L{self.lib}", "-lLeaf")
        graph = self.graph(directories=["Program/lib"])
        self.assertEqual("PASS", graph["status"])
        self.assertEqual([["Program/lib/libLeaf.so.1", "Program/lib/libBranch.so",
                           "Program/lib/libLeaf.so.1"]], graph["cycles"])


if __name__ == "__main__":
    unittest.main()
