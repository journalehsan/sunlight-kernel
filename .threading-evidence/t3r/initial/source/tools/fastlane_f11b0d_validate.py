#!/usr/bin/env python3
"""Retained final-source grant replay. Missing races always keep F1.1B-0 BLOCKED."""
import argparse
from datetime import datetime, timezone
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from fastlane_f11b0c_validate import assess_native, SERVICE_FLAGS

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "target/fastlane-f11b0d-evidence"


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def sources():
    paths = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard"], cwd=ROOT, text=True
    ).splitlines()
    return {p: digest(ROOT / p) for p in sorted(set(paths))
            if (ROOT / p).is_file() and (p.endswith((".rs", ".toml", ".ld", ".py", ".sh", ".S"))
                                         or p in ("Cargo.lock", "limine.cfg", "limine.conf"))}


def assess_races(raw, cpus):
    errors = []
    commit = re.findall(r"\[GRANT-D\] case=exit-after-commit checkpoint=commit issuer=(\d+) issuer_inc=(\d+) recipient=(\d+) incarnation=(\d+) vm=(\d+) session_generation=none grant_batch=postcommit count=2 cpu=(\d+)", raw)
    exits = re.findall(r"\[GRANT-D\] case=(exit-before-commit|exit-after-commit) checkpoint=signal-before-exit issuer_driver=(\d+) driver_inc=(\d+) recipient=(\d+) incarnation=(\d+) vm=(\d+) session_generation=none inventory=(\d+) cpu=(\d+)", raw)
    reaped = re.findall(r"\[GRANT-D\] checkpoint=reaped recipient=(\d+) incarnation=(\d+) exit=(-?\d+) cpu=(\d+)", raw)
    markers = [
        "[GRANT-D] case=exit-before-commit checkpoint=uac-resumed result=aborted",
        "[GRANT-D] case=exit-after-commit checkpoint=uac-resumed result=committed",
        "[GRANT-D] driver COMPLETE precommit=aborted postcommit=retired stale_reply=excluded",
        "[GRANT-D] lifecycle COMPLETE precommit_commits=0 postcommit_commits=2 inventory=0 reaped=3 pid_reuse=NOT_FORCED restart=NOT_FORCED",
    ]
    if any(raw.count(m) != 1 for m in markers) or len(commit) != 1 or len(exits) != 2 or len(reaped) != 3:
        errors.append("missing or duplicate synchronization checkpoints")
    if len(commit) == 1 and len(exits) == 2:
        c = list(map(int, commit[0]))
        cases = {r[0]: list(map(int, r[1:])) for r in exits}
        if set(cases) != {"exit-before-commit", "exit-after-commit"}:
            errors.append("missing independent exit case")
        else:
            pre, post = cases["exit-before-commit"], cases["exit-after-commit"]
            if pre[2] != 0xF4F1 or post[2] != 0xF4F2 or pre[5] != 0 or post[5] != 2:
                errors.append("incorrect recipient or boundary inventory")
            if pre[0] != 0xF4F0 or post[0] != 0xF4F0 or c[2:5] != post[2:5]:
                errors.append("process/VM incarnation mismatch")
            dead = {int(r[0]): int(r[1]) for r in reaped}
            if pre[1] != post[1] or dead != {pre[0]: pre[1], pre[2]: pre[3], post[2]: post[3]}:
                errors.append("reaped incarnation differs from synchronized participant")
            if min(c[0:5] + pre[0:5] + post[0:5]) <= 0 or max(c[-1], pre[-1], post[-1]) >= cpus:
                errors.append("invalid identity or CPU")
            checkpoints = ["case=exit-after-commit checkpoint=commit", "case=exit-after-commit checkpoint=signal-before-exit", markers[1], markers[2], markers[3]]
            positions = [raw.find(m) for m in checkpoints]
            if min(positions) < 0 or positions != sorted(positions):
                errors.append("postcommit critical ordering not observed")
            if raw.find("case=exit-before-commit checkpoint=signal-before-exit") >= raw.find(markers[0]):
                errors.append("precommit critical ordering not observed")
    if {int(r[0]) for r in reaped} != {0xF4F0, 0xF4F1, 0xF4F2}:
        errors.append("missing actual reaper transition")
    for pid, incarnation, code, cpu in reaped:
        if int(incarnation) == 0 or int(cpu) >= cpus or (int(code) == 0) != (int(pid) == 0xF4F0):
            errors.append("invalid terminal process observation")
    accounting = []
    for line in raw.splitlines():
        if "[GRANT-D] accounting phase=" in line:
            row = dict(re.findall(r"(\w+)=(\S+)", line))
            accounting.append({k: int(v) if v.isdigit() else v for k, v in row.items()})
    if [a.get("phase") for a in accounting] != ["start", "final"]:
        errors.append("missing broker/memory snapshots")
    for a in accounting:
        if a["broker_live"] != sum(a[k] for k in ("ipc", "shm", "pending", "delivered", "legacy")):
            errors.append("overlapping broker inventory")
        if a["history_bytes"] != 8 * a["history"] or a["history_capacity_bytes"] != 8 * a["history_capacity"]:
            errors.append("history byte mismatch")
    if any(m in raw for m in ("PANIC", "panicked at", "[GRANT-C] client FAIL", "[FAULT]")):
        errors.append("guest failure")
    status = ("FORCED_AND_FAILED" if exits else "NOT_RUN") if errors else "FORCED_AND_PASSED"
    forced = {row[0] for row in exits}
    tables = [dict((k, int(v) if v.isdigit() else v) for k, v in re.findall(r"(\w+)=(\S+)", line))
              for line in raw.splitlines() if "[GRANT-D] tables phase=" in line]
    if [row.get("phase") for row in tables] != ["start", "final"]:
        errors.append("missing table capacity/record-size observations")
        status = "FORCED_AND_FAILED" if exits else "NOT_RUN"
    return {"classification": status, "errors": errors, "table_metadata": tables,
            "recipient_exit_before_commit": status if "exit-before-commit" in forced else "NOT_FORCED",
            "recipient_exit_after_commit_before_delivery": status if "exit-after-commit" in forced and commit else "NOT_FORCED",
            "checkpoints": {"commit": commit, "exit": exits, "reaped": reaped},
            "accounting": accounting, "session_races": "BLOCKED", "pid_reuse": "NOT_FORCED",
            "service_restart": "NOT_FORCED", "global_reconciliation": "BLOCKED"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checks", action="store_true")
    parser.add_argument("--native", action="store_true")
    parser.add_argument("--cpus", nargs="+", type=int, default=[1, 2, 4, 12])
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    if any(c not in (1, 2, 4, 12) for c in args.cpus) or args.timeout <= 0:
        parser.error("use 1/2/4/12 CPUs and a positive timeout")
    out = BASE / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    out.mkdir(parents=True)
    summary = {"phase": "F1.1B-0D", "raw_directory": str(out.relative_to(ROOT)),
               "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
               "source_sha256": sources(), "checks": [], "native": [],
               "gates": {"A": "VERIFIED NATIVE", "B": "PARTIAL", "F1.1B-0": "BLOCKED", "C": "BLOCKED"},
               "remaining_blockers": ["Account commit/revocation/storage publication serialization",
                                      "native PID reuse", "service restart with outstanding authority",
                                      "remaining lifecycle interleavings", "global authority/allocator reconciliation",
                                      "bounded token history availability budget and reboot boundary"]}
    summary["tool_versions"] = {
        "rustc": subprocess.check_output(["rustc", "-vV"], text=True),
        "cargo": subprocess.check_output(["cargo", "-V"], text=True),
        "qemu": subprocess.check_output(["qemu-system-x86_64", "--version"], text=True),
    }
    summary["build_environment"] = {k: os.environ.get(k) for k in
                                    ("CARGO_INCREMENTAL", "CARGO_TARGET_DIR", "RUSTFLAGS", "SUNLIGHT_CAP_EXEC_PROBE")}
    for p in summary["source_sha256"]:
        dest = out / "source" / p
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / p, dest)
    (out / "worktree.patch").write_bytes(subprocess.check_output(["git", "diff", "HEAD"], cwd=ROOT))

    def save():
        for path in (out / "validation-summary.json", BASE / "validation-summary.json"):
            path.write_text(json.dumps(summary, indent=2) + "\n")

    def check(name, command, env=None, timeout=1200):
        start = time.monotonic()
        with (out / f"{name}.log").open("w") as log:
            log.write(json.dumps({"command": command, "environment": env or {}}) + "\n")
            log.flush()
            try:
                code = subprocess.run(command, cwd=ROOT, env={**os.environ, **(env or {})},
                                      stdout=log, stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        row = {"name": name, "command": command, "environment": env or {}, "exit_code": code,
               "classification": "PASS" if code == 0 else "FAIL", "seconds": time.monotonic() - start,
               "log": str((out / f"{name}.log").relative_to(ROOT))}
        summary["checks"].append(row)
        print(name, row["classification"], flush=True)
        save()
        return code == 0

    try:
        if args.checks:
            host = ["cargo", "test", "--locked", "--offline", "--target", "x86_64-unknown-linux-gnu"]
            check("kernel-host", host + ["-p", "sunlight-kernel", "--lib", "--", "--test-threads=1"])
            packages = ["sunlight-ipc", "sunlight-uac", "sunlight-sessiond", "sunlight-fs", "sunlightd",
                        "sunlight-audiod", "sunlight-elf", "sunlight-fastlane-ring", "sunlight-fastlane-doorbell"]
            check("services-host", host + sum((["-p", p] for p in packages), []) + ["--lib"])
            check("scheduler-host", ["python3", "tools/test_scheduler_host.py"])
            check("evidence-parser", ["python3", "-m", "unittest", "discover", "-s", "tools", "-p", "test_fastlane_f11b0*.py"])
            check("kernel-target", ["cargo", "check", "--locked", "--offline", "-p", "sunlight-kernel", "--bin", "sunlight-kernel"])
            check("strict-clippy", ["cargo", "clippy", "--locked", "--offline", "--target", "x86_64-unknown-linux-gnu", "-p", "sunlight-kernel", "--lib", "--", "-D", "warnings"])
            check("media-host", host + ["-p", "sunlight-media", "--lib"])
            check("workspace-format", ["cargo", "fmt", "--all", "--", "--check"])
            check("diff-check", ["git", "diff", "--check"])
            check("miri-availability", ["cargo", "miri", "--version"])
        if args.native:
            if check("production-services", ["cargo", "build", "--locked", "--offline", "-p", "sunlight-uac", "-p", "sunlight-vfs-server", "--release", "--features", "sunlight-uac/production-grant-test"], env={"RUSTFLAGS": SERVICE_FLAGS}) and check("production-kernel", ["cargo", "build", "--locked", "--offline", "-p", "sunlight-kernel", "--bin", "sunlight-kernel", "--features", "production_grant_test"]):
                summary["embedded_services_sha256"] = {p: digest(ROOT / "target/x86_64-unknown-none/release" / p) for p in ("uac_service", "sunlight-vfs-server", "grant-probe", "sunlight-init", "sunlightd")}
                with tempfile.TemporaryDirectory(prefix="f11b0d-") as scratch:
                    elf, iso = Path(scratch) / "production.elf", Path(scratch) / "production.iso"
                    shutil.copy2(ROOT / "limine.conf", Path(scratch) / "limine.conf")
                    if check("strip-debug", ["objcopy", "--strip-debug", str(ROOT / "target/x86_64-unknown-none/debug/sunlight-kernel"), str(elf)]) and check("production-iso", ["bash", "tools/make_hybrid_iso.sh", str(elf), str(iso), str(ROOT / "target/limine"), scratch]):
                        summary["elf_sha256"], summary["iso_sha256"] = digest(elf), digest(iso)
                        for cpu in args.cpus:
                            serial = out / f"production-{cpu}cpu.serial.log"
                            serial.touch()
                            command = ["qemu-system-x86_64", "-accel", "tcg,thread=multi", "-cpu", "max", "-smp", str(cpu), "-m", "1024", "-cdrom", str(iso), "-serial", f"file:{serial}", "-display", "none", "-nic", "none", "-no-reboot", "-no-shutdown", "-device", "virtio-rng-pci,disable-modern=on", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]
                            start = time.monotonic()
                            with (out / f"production-{cpu}cpu.qemu.log").open("w") as log:
                                proc = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
                                try:
                                    while proc.poll() is None and time.monotonic() - start < args.timeout:
                                        raw = serial.read_text(errors="replace")
                                        if any(m in raw for m in ("[GRANT-C] COMPLETE PASS", "[GRANT-C] client FAIL", "PANIC")):
                                            break
                                        time.sleep(0.25)
                                finally:
                                    if proc.poll() is None:
                                        proc.terminate()
                                        try:
                                            proc.wait(timeout=5)
                                        except subprocess.TimeoutExpired:
                                            proc.kill()
                                    code = proc.wait()
                            raw = serial.read_text(errors="replace")
                            summary["native"].append({"cpus": cpu, "command": command, "exit_code": code,
                                                      "seconds": time.monotonic() - start,
                                                      "serial": str(serial.relative_to(ROOT)), "serial_sha256": digest(serial),
                                                      "delegation": assess_native(raw, cpu), "races": assess_races(raw, cpu)})
                            print(f"native-{cpu}cpu", summary["native"][-1]["races"]["classification"], flush=True)
                            save()
                        # The ISO contains the exact stripped ELF. Retain one
                        # archive to avoid duplicating the embedded apps/media.
                        for image in (iso,):
                            with image.open("rb") as source, gzip.open(out / (image.name + ".gz"), "wb", compresslevel=1) as target:
                                shutil.copyfileobj(source, target)
                        archive = out / "production.iso.gz"
                        with gzip.open(archive, "rb") as stream:
                            assert hashlib.file_digest(stream, "sha256").hexdigest() == summary["iso_sha256"]
                        summary["image_archive"] = {"path": str(archive.relative_to(ROOT)), "sha256": digest(archive), "elf_inside_iso": "/boot/sunlight-kernel.elf"}
        summary["source_sha256_after"] = sources()
        summary["source_hashes_unchanged"] = summary["source_sha256"] == summary["source_sha256_after"]
    finally:
        save()
        print("Evidence:", out.relative_to(ROOT), flush=True)
    if not summary.get("source_hashes_unchanged") or any(r["races"]["errors"] or r["delegation"]["classification"] != "PASS" for r in summary["native"]) or any(c["exit_code"] for c in summary["checks"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
