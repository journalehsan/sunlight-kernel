# Qalculate SunApp staging

From the workspace root, use the retained host build and its pinned package
snapshot. The staging tool needs Python 3.11+, GNU readelf, and the matching
pacman package set for provenance, license files and library symlinks. Package
inputs must match their recorded hashes. The executable is copied unchanged
from the original build.

```sh
snapshot=target/helios-qalculate
mkdir -p "$snapshot/closure"
python3 tools/helios-dynamic-elf/stage_qalculate.py "$snapshot" "$snapshot/closure/candidate"
```

The output contains `Qalculate.sunapp` and `dependency-graph.json`.
`Licenses/provenance.json` records package versions, declared licenses,
available package license files, and source paths. Packages without license
files in the installed package directory still need a distribution review.

`tools/helios-dynamic-elf/resolve_bundle.py` also works directly on other
Helios SunApps. Pass their guest installation path with `--install`, each
bundle-relative private library directory with `--library-dir`, runtime
plugin ELF roots with `--plugin`, and the runtime's interpreter binding
with `--interpreter GUEST_PATH=BUNDLE_PATH`. It reads ELF metadata with
readelf, reports required version definitions, and returns nonzero for
incomplete closure. On an ELF resolution failure it writes the graph and
diagnostics to the requested output path.

The private directories model explicit guest library search directories.
The resolver uses bundle-local RPATH/RUNPATH entries and expands
`$ORIGIN` at the declared guest location. It records and excludes search
paths outside the bundle; it never searches the host loader cache, host
default directories, or glibc hardware capability directories. It checks
version definitions, not runtime symbol binding or CPU feature availability.
Plugin roots must be declared. Successful static validation does not install
the bundle, configure the guest loader, or prove that runtime-loaded
components outside those roots are unnecessary.

The Qalculate candidate keeps the existing manifest and private
`Program/lib` and `Program/plugins` trees. The intended guest launch needs
a runtime-approved interpreter binding and child-only library search path
`/Applications/Qalculate.sunapp/Program/lib`, plus Wayland QPA selection
and the existing fixture socket at `/sunlight/native-unix/wayland-fixture`.
The guest settings are pending validation. See
[the audit](../docs/audit/helios-qalculate.md) for the launch blockers.
