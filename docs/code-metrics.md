# SunlightOS Code Metrics

Run `./count-lines.py` (or the compatibility entry point `./count-lines.sh`). Use `--details`, `--subsystems`, `--json`, `--markdown report.md`, `--mode filesystem`, `--include-vendored`, `--save-snapshot [path]`, `--compare OLD.json NEW.json`, and `--benchmark` as needed. JSON goes to stdout without human text; diagnostics go to stderr. `--workers N` is accepted for automation compatibility; the current streaming counter remains sequential because small-file threading has no established benefit.

## Definitions

- Physical lines count records separated by LF; a final unterminated record counts once. CRLF is one separator. Non-blank means the byte record contains at least one non-whitespace byte. Invalid UTF-8 remains countable; NUL-containing files are treated as binary and excluded.
- Source contains executable language extensions only. TOML configuration and Markdown documentation have separate totals. DOCX is counted as documents only; the tool does not invent a line estimate.
- Default inventory is `git ls-files`, including tracked files regardless of working-tree changes but excluding untracked files. `--mode filesystem` includes untracked regular files. Symlinks are skipped. `tracked_files` is the raw Git inventory size; `scanned_files` counts eligible readable metric files.
- Build/cache directories, archived `.threading-evidence/` source snapshots, and generated/minified files are excluded. `vendor/`, `third_party/`, `third-party/`, and `external/` are excluded by default; `--include-vendored` includes them and exposes first-party/third-party source line totals. The tracked-file total is still the unfiltered Git inventory.
- Rust kernel means `kernel/`; non-kernel Rust is the remainder of included Rust. The ratio describes code distribution, not isolation. Cargo manifest count means actual eligible `Cargo.toml` files; workspace members count listed member directories with a manifest.
- Each source path has one primary subsystem owner. Explicit roots appear in `count-lines.py`; unmatched source paths are `Unclassified`. `sunlight-files/` is owned by Filesystem and storage, with Applications as a secondary conceptual association. Secondary associations are metadata only and are not added to aggregate totals.
- Test statistics are static Rust test-attribute/path indicators, not test execution or pass counts. Markdown lines exclude DOCX; DOCX line count is deliberately `null`.

## Snapshots and performance

Snapshots use JSON schema version 1 and contain dated aggregate source, Rust, kernel/non-kernel, documentation, file, workspace-member, and subsystem line counts. They are written only when requested and are never committed automatically. Compare two snapshots with `--compare OLD.json NEW.json`.

The normal scan performs one Git inventory and streams each eligible text file once. The benchmark reports elapsed scan time and process peak RSS where the platform exposes it. No cache is used, so edits and deletions are reflected on each run. The compatibility shell script delegates directly to Python 3.
