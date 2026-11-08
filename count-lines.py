#!/usr/bin/env python3
"""Fast, deterministic source metrics for SunlightOS."""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import fnmatch
import json
import os
import re
import subprocess
import sys
import time
import zipfile
from pathlib import Path
from typing import Any, Iterable

SCHEMA_VERSION = 1
EXCLUDED_DIRS = {".git", ".threading-evidence", "target", "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", "node_modules", "dist", "build", ".venv", "venv"}
EXCLUDED_PREFIXES = ("vendor/", "third_party/", "third-party/", "external/")
EXTENSIONS = {
    ".rs": ("Rust", "source"), ".pas": ("Pascal", "source"), ".pp": ("Pascal", "source"), ".p": ("Pascal", "source"),
    ".inc": ("Pascal", "source"), ".asm": ("Assembly", "source"), ".s": ("Assembly", "source"),
    ".bat": ("DOS Batch", "source"), ".cmd": ("DOS Batch", "source"), ".py": ("Python", "source"),
    ".sh": ("Shell", "source"), ".bash": ("Shell", "source"), ".js": ("JavaScript", "source"),
    ".jsx": ("JavaScript", "source"), ".mjs": ("JavaScript", "source"), ".ts": ("TypeScript", "source"),
    ".tsx": ("TypeScript", "source"), ".c": ("C/C++", "source"), ".h": ("C/C++", "source"),
    ".cc": ("C/C++", "source"), ".cpp": ("C/C++", "source"), ".cxx": ("C/C++", "source"),
    ".hpp": ("C/C++", "source"), ".hh": ("C/C++", "source"), ".toml": ("TOML", "config"),
    ".md": ("Markdown", "documentation"), ".markdown": ("Markdown", "documentation"),
}
SUBSYSTEMS = [
    ("Kernel and core", ("kernel/",)),
    ("IPC and FastLane", ("ipc/", "fastlane-ring/", "fastlane/")),
    ("Drivers and devices", ("drivers/", "sunlight-virtio/", "sunlight-audio/")),
    ("Networking and fetch", ("sunlight-net/", "sunlight-tls/", "sunlight-fetch/", "sunlight-api-lab/")),
    ("Filesystem and storage", ("sunlight-fs/", "sunlight-files/", "sunlight-fat/", "sunlight-block/", "sunlight-kv/")),
    ("Desktop and UI", ("sunlight-ui/", "sunlight-top/", "sunlight-tty/", "sunlight-tui/", "sunlight-edit/", "sunlight-writer/", "sunlight-docview/", "sunlight-docserv/")),
    ("Applications", ("melody-mina/", "sunlight-media/", "sunlight-reminders/", "sunlight-voice-recorder/", "sunlight-files/", "sunlight-js/")),
    ("Rappid Rabbit and Golden Fish", ("rappid-rabbit/", "golden-fish/")),
    ("Helios and Linux compatibility", ("helios-note/", "compat-linux/", "sunlight-libc/")),
    ("Chronos and DOS compatibility", ("chronos-core/", "sunlight-chronos/", "guest/", "ChronosDosShell.sunapp/", "ChronosMzLab.sunapp/", "ChronosFileLab.sunapp/")),
    ("WiseOwl", ("wiseowl-memory/", "wiseowl-memorydb/", "wiseowl-index/", "wiseowl-brain/", "wiseowl-identity/", "wiseowl-console/")),
    ("Testing and development tools", ("tools/", "sunlight-bench/", "std-proof/")),
    ("Other first-party", ()),
]
SECONDARY_ASSOCIATIONS = {"sunlight-files/": "Applications"}


def tracked_paths(root: Path) -> list[str]:
    proc = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    return [p.decode("utf-8", "surrogateescape") for p in proc.stdout.split(b"\0") if p]


def filesystem_paths(root: Path) -> list[str]:
    paths = []
    for base, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if d not in EXCLUDED_DIRS and not (Path(base) / d).is_symlink())
        for name in files:
            path = Path(base) / name
            if not path.is_symlink():
                paths.append(path.relative_to(root).as_posix())
    return sorted(paths)


def is_generated(path: str) -> bool:
    lower = path.lower()
    parts = lower.split("/")
    return any(p in EXCLUDED_DIRS for p in parts) or any(x in lower for x in (".generated.", ".min.")) or lower.endswith((".pb.rs", "_generated.rs"))


def read_counts(path: Path) -> tuple[int, int, bool, bool]:
    physical = nonblank = 0
    test_indicator = False
    try:
        with path.open("rb") as stream:
            for raw in stream:
                if b"\0" in raw:
                    return 0, 0, True, False
                physical += 1
                if raw.decode("utf-8", errors="replace").strip():
                    nonblank += 1
                if b"#[test]" in raw or b"#[cfg(test)]" in raw:
                    test_indicator = True
        return physical, nonblank, False, test_indicator
    except (OSError, ValueError):
        return 0, 0, True, False


def subsystem_for(path: str) -> str:
    for name, prefixes in SUBSYSTEMS:
        if any(path.startswith(prefix) for prefix in prefixes):
            return name
    return "Unclassified"


def cargo_info(root: Path, paths: list[str]) -> tuple[int, int, list[str]]:
    manifests = [p for p in paths if Path(p).name == "Cargo.toml" and not is_generated(p)]
    members: set[str] = set()
    for rel in manifests:
        if rel != "Cargo.toml":
            continue
        try:
            text = (root / rel).read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        try:
            import tomllib
            members.update(tomllib.loads(text).get("workspace", {}).get("members", []))
        except (ImportError, ValueError):
            match = re.search(r"(?ms)^\[workspace\]\s*(.*?)(?=^\[|\Z)", text)
            if match:
                workspace_text = match.group(1)
                member_block = re.search(r"(?ms)^\s*members\s*=\s*(\[.*?\])", workspace_text)
                if member_block:
                    members.update(re.findall(r"['\"]([^'\"]+)['\"]", member_block.group(1)))
    expanded = set()
    for member in members:
        if any(c in member for c in "*?["):
            expanded.update(
                str(Path(p).parent)
                for p in manifests
                if fnmatch.fnmatch(str(Path(p).parent), member.rstrip("/") or ".")
                or fnmatch.fnmatch(str(Path(p).parent) + "/", member)
            )
        else:
            expanded.add(member.rstrip("/"))
    found = sorted(p for p in manifests if str(Path(p).parent) in expanded)
    return len(manifests), len(found), found


def collect(root: Path, mode: str = "git", workers: int = 0, include_vendored: bool = False) -> dict[str, Any]:
    paths = tracked_paths(root) if mode == "git" else filesystem_paths(root)
    file_rows: list[dict[str, Any]] = []
    unreadable = binary = 0
    for rel in paths:
        vendored = rel.lower().startswith(EXCLUDED_PREFIXES)
        if is_generated(rel) or (vendored and not include_vendored):
            continue
        path = root / rel
        if not path.is_file() or path.is_symlink():
            continue
        ext = Path(rel).suffix.lower()
        kind = EXTENSIONS.get(ext)
        if Path(rel).name == "Cargo.toml":
            kind = ("TOML", "config")
        if not kind and ext != ".docx":
            continue
        if ext == ".docx":
            file_rows.append({"path": rel, "language": "Word documents", "category": "documentation_count_only", "lines": 0, "nonblank_lines": 0,
                              "third_party": False, "subsystem": subsystem_for(rel), "test_indicator": False})
            continue
        lines, nonblank, invalid, test_indicator = read_counts(path)
        if invalid:
            if path.exists() and path.stat().st_size:
                binary += 1
            else:
                unreadable += 1
            continue
        lang, category = kind if kind else ("Word documents", "documentation_count_only")
        row = {"path": rel, "language": lang, "category": category, "lines": lines, "nonblank_lines": nonblank,
               "third_party": vendored, "subsystem": subsystem_for(rel), "test_indicator": test_indicator}
        file_rows.append(row)
    # Keep the option explicit; sequential streaming wins for typical small source files.
    cargo_count, workspace_crates, workspace_paths = cargo_info(root, paths)
    by_lang: dict[str, dict[str, int]] = collections.defaultdict(lambda: {"files": 0, "lines": 0, "nonblank_lines": 0})
    by_subsystem: dict[str, dict[str, int]] = collections.defaultdict(lambda: {"files": 0, "lines": 0, "nonblank_lines": 0})
    totals: dict[str, dict[str, int]] = collections.defaultdict(lambda: {"files": 0, "lines": 0, "nonblank_lines": 0})
    source_files = []
    docs_lines = docs_files = config_lines = config_files = third_party_lines = first_party_lines = 0
    for row in file_rows:
        lang, cat, lines, nb = row["language"], row["category"], row["lines"], row["nonblank_lines"]
        by_lang[lang]["files"] += 1; by_lang[lang]["lines"] += lines; by_lang[lang]["nonblank_lines"] += nb
        totals[cat]["files"] += 1; totals[cat]["lines"] += lines; totals[cat]["nonblank_lines"] += nb
        if cat == "source":
            source_files.append(row)
            by_subsystem[row["subsystem"]]["files"] += 1; by_subsystem[row["subsystem"]]["lines"] += lines; by_subsystem[row["subsystem"]]["nonblank_lines"] += nb
            if row["third_party"]: third_party_lines += lines
            else: first_party_lines += lines
        elif cat == "documentation": docs_lines += lines; docs_files += 1
        elif cat == "config": config_lines += lines; config_files += 1
    rust = by_lang.get("Rust", {"files": 0, "lines": 0, "nonblank_lines": 0})
    kernel_rows = [r for r in source_files if r["language"] == "Rust" and r["path"].startswith("kernel/")]
    kernel = {"files": len(kernel_rows), "lines": sum(r["lines"] for r in kernel_rows), "nonblank_lines": sum(r["nonblank_lines"] for r in kernel_rows)}
    nonkernel = {k: rust[k] - kernel[k] for k in rust}
    first_party_rust_lines = sum(r["lines"] for r in source_files if r["language"] == "Rust" and not r["third_party"])
    test_rows = [r for r in source_files if r["language"] == "Rust" and ("tests/" in r["path"] or r["path"].endswith("_test.rs") or r["test_indicator"])]
    docs_only = by_lang.get("Markdown", {"files": 0, "lines": 0, "nonblank_lines": 0})
    modules: dict[str, int] = collections.defaultdict(int)
    for row in source_files:
        if row["language"] == "Rust":
            modules[str(Path(row["path"]).parent)] += row["lines"]
    return {
        "schema_version": SCHEMA_VERSION, "date": dt.date.today().isoformat(), "project_day": project_day(root), "mode": mode,
        "tracked_files": len(paths) if mode == "git" else None, "scanned_files": len(file_rows), "unreadable_or_binary_files": unreadable + binary,
        "binary_files": binary, "unreadable_files": unreadable, "languages": dict(sorted(by_lang.items())),
        "categories": dict(sorted(totals.items())), "source": totals.get("source", {"files": 0, "lines": 0, "nonblank_lines": 0}),
        "documentation": {"markdown": docs_only, "docx_files": by_lang.get("Word documents", {}).get("files", 0), "docx_line_count": None},
        "configuration": {"files": config_files, "lines": config_lines}, "rust": {"total": rust, "kernel": kernel, "non_kernel": nonkernel,
            "kernel_percent": round(100 * kernel["lines"] / rust["lines"], 2) if rust["lines"] else 0,
            "first_party_lines": first_party_rust_lines,
            "kernel_percent_of_first_party": round(100 * kernel["lines"] / first_party_rust_lines, 2) if first_party_rust_lines else 0,
            "non_kernel_to_kernel_ratio": round(nonkernel["lines"] / kernel["lines"], 2) if kernel["lines"] else None},
        "cargo": {"manifest_count": cargo_count, "workspace_member_count": workspace_crates, "workspace_manifests": workspace_paths},
        "subsystems": {k: by_subsystem.get(k, {"files": 0, "lines": 0, "nonblank_lines": 0}) for k, _ in SUBSYSTEMS} | {"Unclassified": by_subsystem.get("Unclassified", {"files": 0, "lines": 0, "nonblank_lines": 0})},
        "subsystem_secondary_associations": SECONDARY_ASSOCIATIONS,
        "distribution": {"largest_rust_files": sorted((r for r in source_files if r["language"] == "Rust"), key=lambda r: (-r["lines"], r["path"]))[:10],
            "largest_rust_modules": [{"module": name, "lines": lines} for name, lines in sorted(modules.items(), key=lambda item: (-item[1], item[0]))[:10]],
            "largest_source_files": sorted(source_files, key=lambda r: (-r["lines"], r["path"]))[:10],
            "largest_subsystem": max(by_subsystem.items(), key=lambda kv: kv[1]["lines"])[0] if by_subsystem else None,
            "largest_classified_subsystem": max(((k, v) for k, v in by_subsystem.items() if k != "Unclassified"), key=lambda kv: kv[1]["lines"])[0] if any(k != "Unclassified" for k in by_subsystem) else None,
            "source_to_markdown_ratio": round(totals.get("source", {}).get("lines", 0) / docs_lines, 2) if docs_lines else None,
            "first_party_lines": first_party_lines, "third_party_lines": third_party_lines},
        "tests_static_estimate": {"rust_files_with_test_indicators": len(test_rows), "test_source_lines": sum(r["lines"] for r in test_rows),
            "integration_test_files": sum("tests/" in r["path"] for r in test_rows), "directories": sorted({str(Path(r["path"]).parent) for r in test_rows})},
    }


def project_day(root: Path) -> dict[str, Any]:
    try:
        history = subprocess.run(["git", "-C", str(root), "log", "--reverse", "--format=%ad", "--date=short"], capture_output=True, text=True, check=True).stdout
        first = history.splitlines()[0] if history.splitlines() else ""
    except (OSError, subprocess.CalledProcessError):
        first = "2026-06-06"
    start = dt.date.fromisoformat(first) if first else dt.date(2026, 6, 6)
    today = dt.date.today()
    return {"start_date": start.isoformat(), "as_of": today.isoformat(), "day": max(1, (today - start).days + 1)}


def snapshot(data: dict[str, Any]) -> dict[str, Any]:
    return {"schema_version": SCHEMA_VERSION, "date": data["date"], "project_day": data["project_day"], "metrics": {
        "source_lines": data["source"]["lines"], "rust_lines": data["rust"]["total"]["lines"], "kernel_lines": data["rust"]["kernel"]["lines"],
        "non_kernel_lines": data["rust"]["non_kernel"]["lines"], "documentation_lines": data["documentation"]["markdown"]["lines"],
        "source_files": data["source"]["files"], "cargo_crates": data["cargo"]["workspace_member_count"],
        "subsystems": {k: v["lines"] for k, v in data["subsystems"].items()}}}


def compare(old: dict[str, Any], new: dict[str, Any]) -> dict[str, Any]:
    a, b = old["metrics"], new["metrics"]
    keys = ("source_lines", "rust_lines", "kernel_lines", "non_kernel_lines", "documentation_lines", "source_files", "cargo_crates")
    changes = {k: {"absolute": b[k] - a[k], "percent": round(100 * (b[k] - a[k]) / a[k], 2) if a[k] else None} for k in keys}
    subsystems = sorted(set(a["subsystems"]) | set(b["subsystems"]))
    changes["subsystems"] = {k: {"absolute": b["subsystems"].get(k, 0) - a["subsystems"].get(k, 0), "percent": round(100 * (b["subsystems"].get(k, 0) - a["subsystems"].get(k, 0)) / a["subsystems"].get(k, 0), 2) if a["subsystems"].get(k, 0) else None} for k in subsystems}
    return {"old_date": old["date"], "new_date": new["date"], "changes": changes}


def human(data: dict[str, Any], details: bool = False, subsystems: bool = False) -> str:
    rust = data["rust"]
    out = [f"SunlightOS Code Metrics — project day {data['project_day']['day']} ({data['date']})",
           f"Source: {data['source']['lines']:,} lines across {data['source']['files']:,} files; tracked files: {data['tracked_files'] if data['tracked_files'] is not None else 'filesystem mode':,}" if isinstance(data['tracked_files'], int) else f"Source: {data['source']['lines']:,} lines across {data['source']['files']:,} files; filesystem mode",
           f"Rust: {rust['total']['lines']:,} lines / {rust['total']['files']:,} files; kernel {rust['kernel']['lines']:,} ({rust['kernel_percent_of_first_party']:.2f}% of first-party Rust), non-kernel {rust['non_kernel']['lines']:,} ({rust['non_kernel_to_kernel_ratio']}:1 non-kernel:kernel)",
           f"Cargo: {data['cargo']['manifest_count']:,} manifests; {data['cargo']['workspace_member_count']:,} workspace members",
           f"Docs: Markdown {data['documentation']['markdown']['lines']:,} lines / {data['documentation']['markdown']['files']:,} files; DOCX {data['documentation']['docx_files']:,} files (line count not estimated)",
           f"Static tests: {data['tests_static_estimate']['rust_files_with_test_indicators']:,} Rust files with indicators; not executed"]
    if subsystems or details:
        if details:
            out.append("Languages, configuration, and documentation:")
            out.extend(f"  {name}: {values['lines']:,} lines / {values['files']:,} files" for name, values in data["languages"].items())
        out.append("Subsystem source lines:")
        out.extend(f"  {name}: {v['lines']:,} lines / {v['files']:,} files" for name, v in data["subsystems"].items() if v["files"])
    if details:
        out.append("Largest Rust modules:")
        out.extend(f"  {row['module']}: {row['lines']:,}" for row in data["distribution"]["largest_rust_modules"])
        out.append("Largest Rust files:")
        out.extend(f"  {row['path']}: {row['lines']:,}" for row in data["distribution"]["largest_rust_files"])
    return "\n".join(out)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", action="store_true", help="compact default summary")
    parser.add_argument("--details", action="store_true", help="include subsystem and largest-file detail")
    parser.add_argument("--subsystems", action="store_true", help="show source totals by subsystem")
    parser.add_argument("--json", action="store_true", help="emit JSON only")
    parser.add_argument("--markdown", metavar="PATH", help="write a Markdown report")
    parser.add_argument("--mode", choices=("git", "filesystem"), default="git")
    parser.add_argument("--include-vendored", action="store_true", help="include vendor/third_party/external trees")
    parser.add_argument("--workers", type=int, default=0, help="reserved; sequential streaming is used for small files")
    parser.add_argument("--save-snapshot", nargs="?", const="sunlightos-metrics.json", metavar="PATH")
    parser.add_argument("--compare", nargs=2, metavar=("OLD.json", "NEW.json"))
    parser.add_argument("--benchmark", action="store_true", help="measure collection runtime and peak RSS")
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parent
    try:
        if args.compare:
            result = compare(*(json.loads(Path(p).read_text(encoding="utf-8")) for p in args.compare))
            print(json.dumps(result, indent=2, sort_keys=True) if args.json else json.dumps(result, indent=2, sort_keys=True))
            return 0
        begin = time.perf_counter()
        data = collect(root, args.mode, max(0, args.workers), args.include_vendored)
        elapsed = time.perf_counter() - begin
        if args.benchmark:
            try:
                import resource
                peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
            except ImportError:
                peak = None
            data["benchmark"] = {"seconds": round(elapsed, 4), "peak_rss_kib": peak, "scanned_files": data["scanned_files"]}
        if args.save_snapshot:
            Path(args.save_snapshot).write_text(json.dumps(snapshot(data), indent=2, sort_keys=True) + "\n", encoding="utf-8")
        if args.json:
            print(json.dumps(data, indent=2, sort_keys=True))
        else:
            report = human(data, args.details, args.subsystems)
            print(report)
            if args.benchmark:
                print(f"Scan: {elapsed:.3f}s; peak RSS: {data['benchmark']['peak_rss_kib']} KiB")
        if args.markdown:
            Path(args.markdown).write_text("# SunlightOS Code Metrics\n\n```text\n" + human(data, True, True) + "\n```\n", encoding="utf-8")
        return 0
    except (OSError, subprocess.CalledProcessError, ValueError, KeyError) as exc:
        print(f"count-lines: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
