#!/usr/bin/env python3
"""Final-source cooperative ring checks. Never claims native FastLane readiness."""
import datetime
import hashlib
import json
import math
from pathlib import Path
import platform
import resource
import subprocess


ROOT = Path(__file__).resolve().parents[1]
SOURCES = [
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
    "fastlane-ring/Cargo.toml", "fastlane-ring/src/lib.rs",
    "fastlane-ring/src/leases.rs", "fastlane-ring/src/tests.rs",
    "fastlane-ring/src/lease_tests.rs", "fastlane-ring/examples/lease_bench.rs",
    "fastlane-doorbell/Cargo.toml", "fastlane-doorbell/src/lib.rs",
    "fastlane-doorbell/src/tests.rs", "tools/fastlane_leases_validate.py",
    "docs/ipc/fastlane-zero-copy-leases.md",
    "docs/ipc/fastlane-security-closure-progress.md",
]


def hashes():
    return {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
            for name in SOURCES}


def main():
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    out = ROOT / "target/fastlane-leases-evidence" / stamp
    out.mkdir(parents=True, exist_ok=False)
    before = hashes()
    checks = {}

    def run(name, command, timeout=120):
        with (out / f"{name}.log").open("w") as log:
            try:
                code = subprocess.run(command, cwd=ROOT, stdout=log,
                                      stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = "TIMEOUT"
        checks[name] = {"command": command, "exit_code": code, "log": f"{name}.log"}
        print(f"{name}: {code}", flush=True)
        return code == 0

    host = ["--locked", "--offline", "--target", "x86_64-unknown-linux-gnu"]
    run("ring-tests", ["cargo", "test", "-p", "sunlight-fastlane-ring", *host])
    run("doorbell-tests", ["cargo", "test", "-p", "sunlight-fastlane-doorbell", *host])
    run("clippy", ["cargo", "clippy", "-p", "sunlight-fastlane-ring", "-p",
                   "sunlight-fastlane-doorbell", "--all-targets", *host, "--", "-D", "warnings"])
    run("baremetal-check", ["cargo", "check", "-p", "sunlight-fastlane-ring", "-p",
                            "sunlight-fastlane-doorbell", "--locked", "--offline",
                            "--target", "x86_64-unknown-none"])
    rust = [name for name in SOURCES if name.endswith(".rs")]
    run("format", ["rustfmt", "--edition", "2021", "--check", *rust])
    run("whitespace", ["git", "diff", "--check", "--", *SOURCES])
    built = run("benchmark-build", ["cargo", "build", "-p", "sunlight-fastlane-ring",
                                    "--example", "lease_bench", "--release", *host])
    metrics = []
    cpu = None
    if built:
        usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
        passed = run("benchmark", [str(ROOT / "target/x86_64-unknown-linux-gnu/release/examples/lease_bench")])
        usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
        cpu = {"user_seconds": usage_after.ru_utime - usage_before.ru_utime,
               "system_seconds": usage_after.ru_stime - usage_before.ru_stime,
               "scope": "whole host benchmark including warmups, checks, clocks and thread creation"}
        if passed:
            try:
                metrics = [json.loads(line) for line in (out / "benchmark.log").read_text().splitlines()]
                expected = {(mode, capacity, payload) for mode in ["copy", "leases"]
                            for capacity in [2, 64] for payload in [64, 256, 1024, 4096]}
                assert len(metrics) == len(expected)
                assert {(m["mode"], m["capacity"], m["payload_bytes"]) for m in metrics} == expected
                for m in metrics:
                    assert m["scope"] == "host_threads" and m["integrity"] == "PASS"
                    assert m["messages"] == 100_000 and m["elapsed_ns"] > 0
                    assert math.isfinite(m["messages_per_second"]) and m["messages_per_second"] > 0
                    assert 0 <= m["single_thread_pair_p50_ns"] <= m["single_thread_pair_p95_ns"] <= m["single_thread_pair_p99_ns"]
                checks["metrics"] = {"exit_code": 0, "rows": len(metrics)}
            except (ValueError, KeyError, TypeError, AssertionError) as error:
                checks["metrics"] = {"exit_code": 1, "error": repr(error)}
                metrics = []
    with (out / "environment.log").open("w") as log:
        log.write(platform.platform() + "\n")
        for command in [["rustc", "-Vv"], ["git", "rev-parse", "HEAD"], ["lscpu"],
                        ["rustup", "component", "list", "--installed"]]:
            subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    unchanged = before == hashes()
    passed = (all(check["exit_code"] == 0 for check in checks.values())
              and len(metrics) == 16 and unchanged)
    summary = {
        "scope": "cooperative zero-copy SPSC leases; host threads only",
        "host_status": "PASS" if passed else "FAIL",
        "source_unchanged": unchanged, "source_sha256": before,
        "checks": checks, "host_metrics": metrics, "benchmark_process_cpu": cpu,
        "unsafe_dynamic_analysis": "NOT_RUN: Miri is not installed; no Loom dependency added",
        "production_fastlane": "BLOCKED",
        "native_cpu_matrix": {
            str(n): {"status": "BLOCKED", "latency_ns": None, "messages_per_second": None,
                     "cpu_overhead": None, "kernel_ipc_baseline": None}
            for n in [2, 4, 12]
        },
        "blockers": ["pending reply/endpoint closure and stale adoption security proof",
                     "restricted mapping permissions, provenance, active revocation and lifetime",
                     "global authority, grant lifecycle, session authority and Doorbell B gates"],
        "limitations": ["no process mappings or authenticated native channel created",
                        "host Doorbell uses fixture identities and a mutex/Condvar",
                        "no native IPC speedup, CPU savings, or hostile-peer isolation claim",
                        "payload clearing protects lease-visible bytes, not hostile raw mapping reads"],
    }
    path = out / "validation-summary.json"
    path.write_text(json.dumps(summary, indent=2) + "\n")
    print(path.relative_to(ROOT), flush=True)
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
