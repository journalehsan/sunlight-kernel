#!/usr/bin/env python3
"""Fresh compatibility images/logs without overwriting previous phase evidence.

Transient images live in /tmp; exact hashes, commands and raw serial stay in
target/fastlane-f11b0e-evidence. Passing D3 never promotes Doorbell Gate B.
"""
import argparse
from datetime import datetime, timezone
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import time

from fastlane_f11b0e_validate import ROOT, BASE, SERVICE_FLAGS, digest, sources, assess_authority_fallback
from fastlane_f13nr2v_validate import validate_native as validate_pin
from test_native_doorbell import validate_native as validate_doorbell


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--section", choices=["authority", "core", "lifecycle", "network", "all"], default="all")
    args = parser.parse_args()
    out = BASE / ("compat-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ"))
    out.mkdir(parents=True)
    summary = {"source_sha256": sources(), "checks": [], "images": {}, "native": [],
               "gates": {"A": "VERIFIED NATIVE", "B": "PARTIAL", "F1.1B-0": "BLOCKED", "C": "BLOCKED"}}

    def save():
        (out / "validation-summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    def check(name, command, env=None, timeout=1200):
        with (out / (name + ".log")).open("w") as log:
            log.write(json.dumps({"command": command, "environment": env or {}}) + "\n")
            log.flush()
            try:
                code = subprocess.run(command, cwd=ROOT, env={**os.environ, **(env or {})}, stdout=log,
                                      stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        summary["checks"].append({"name": name, "command": command, "environment": env or {},
                                  "exit_code": code, "classification": "PASS" if code == 0 else "FAIL",
                                  "log": str((out / (name + ".log")).relative_to(ROOT))})
        print(name, summary["checks"][-1]["classification"], flush=True)
        save()
        return code == 0

    def users(name="ordinary-users", packages=None, features=None):
        cmd = ["cargo", "build", "--locked", "--offline", "--release"]
        for package in packages or ["sunlight-init", "sunlightd", "sunlight-startup-fixture", "sunlight-uac", "sunlight-sessiond", "sunlight-vfs-server"]:
            cmd += ["-p", package]
        if features:
            cmd += ["--features", features]
        return check(name, cmd, {"RUSTFLAGS": SERVICE_FLAGS})

    def image(name, feature, scratch, env=None):
        command = ["cargo", "build", "--locked", "--offline", "-p", "sunlight-kernel", "--bin", "sunlight-kernel", "--features", feature]
        if not check(name + "-kernel", command, env):
            return None
        elf, iso = Path(scratch) / (name + ".elf"), Path(scratch) / (name + ".iso")
        if not check(name + "-strip", ["objcopy", "--strip-debug", str(ROOT / "target/x86_64-unknown-none/debug/sunlight-kernel"), str(elf)]):
            return None
        if not check(name + "-iso", ["bash", "tools/make_hybrid_iso.sh", str(elf), str(iso), str(ROOT / "target/limine"), scratch]):
            return None
        summary["images"][name] = {"feature": feature, "elf_sha256": digest(elf), "iso_sha256": digest(iso),
                                   "retention": "transient image; exact identities and raw execution logs retained",
                                   "embedded_release_sha256": {p.name: digest(p) for p in (ROOT / "target/x86_64-unknown-none/release").iterdir() if p.is_file() and not p.suffix}}
        save()
        return iso

    def native(name, iso, cpu, markers, validator=None, no_entropy=False):
        if iso is None:
            summary["native"].append({"name": name, "cpus": cpu, "classification": "NOT_RUN", "reason": "image build failed"})
            return
        serial = out / (name + ".serial.log")
        serial.touch()
        command = ["qemu-system-x86_64", "-accel", "tcg,thread=multi", "-cpu", "max,-rdrand,-rdseed" if no_entropy else "max", "-smp", str(cpu), "-m", "1024", "-cdrom", str(iso), "-serial", f"file:{serial}", "-display", "none", "-nic", "none", "-no-reboot", "-no-shutdown", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]
        if not no_entropy:
            command += ["-device", "virtio-rng-pci,disable-modern=on"]
        start = time.monotonic()
        with (out / (name + ".qemu.log")).open("w") as log:
            proc = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            try:
                while proc.poll() is None and time.monotonic() - start < 180:
                    raw = serial.read_text(errors="replace")
                    if all(m in raw for m in markers) or "PANIC" in raw or "panicked at" in raw:
                        break
                    time.sleep(.25)
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                code = proc.wait()
        raw = serial.read_text(errors="replace")
        missing = [m for m in markers if m not in raw]
        bad = "PANIC" in raw or "panicked at" in raw
        detail = validator(raw, cpu) if validator else None
        good_detail = detail is None or detail.get("classification", detail.get("status")) in ("FORCED_AND_PASSED", "pass")
        row = {"name": name, "cpus": cpu, "command": command, "exit_code": code, "seconds": time.monotonic() - start,
               "classification": "PASS" if not missing and not bad and good_detail else "FAIL", "missing": missing,
               "validator": detail, "serial": str(serial.relative_to(ROOT)), "serial_sha256": digest(serial)}
        summary["native"].append(row)
        print(name, row["classification"], flush=True)
        save()

    try:
        with tempfile.TemporaryDirectory(prefix="f11b0e-compat-") as scratch:
            shutil.copy2(ROOT / "limine.conf", Path(scratch) / "limine.conf")
            if not users():
                raise RuntimeError("ordinary service build failed")
            if args.section in ("all", "authority"):
                iso = image("authority", "cap_issuance_test", scratch, {"SUNLIGHT_CAP_EXEC_PROBE": "1"})
                native("entropy-unavailable", iso, 1, ["[CAP] bootstrap BLOCKED: secure entropy unavailable; no user authority published"], no_entropy=True)
                for cpu in (1, 2, 4, 12):
                    native(f"authority-{cpu}cpu", iso, cpu, [f"[CAP-R41] COMPLETE delivered_auth_consumed=15 stale_incarnation=denied real_call_cancel=retired pending=0 active_auth_account=0 cpus={cpu}", f"[CAP-PR] lifecycle PASS auth=17 account=0 account_denied=35 post_reap_records=0 late_auth=denied late_account=denied cpus={cpu}", "[GRANT-D] retired-token local_broker recycled_storage=observed repeated_candidate=rejected stuck_source=failed_closed old_lookup=denied active=0", "[EXEC-R2] eight replacements PASS; entering original /bin/true", "[HISTORY-E] boundary capacity=65536 history=65536 active=0 oldest=denied newest=denied extra=TokenHistoryFull existing=usable partial=0 entropy_after_full=unchanged restart=UNSUPPORTED scope=isolated-production-broker cpu=0"], assess_authority_fallback)
            if args.section in ("all", "core"):
                for name, feature, marker, validator in [
                    ("pin-release", "pin_release_test", "[R2V] PASS cpus={cpu} ", validate_pin),
                    ("doorbell", "native_doorbell_test", "[F13NA] PASS ", validate_doorbell),
                    ("scheduler", "scheduler_b_test", "[SCHED-B] PASS cpus={cpu} ", None),
                ]:
                    iso = image(name, feature, scratch)
                    for cpu in (1, 2, 4, 12):
                        native(f"{name}-{cpu}cpu", iso, cpu, [marker.format(cpu=cpu)], validator)
                for name, feature, cpu, expected in [("vm-tlb", "mm2b_smp_test", 12, "mm2b.expected"), ("vm-unmap", "mm2d_munmap_test", 4, "mm2d.expected"), ("vm-protect", "mm2e_mprotect_test", 4, "mm2e.expected")]:
                    iso = image(name, feature, scratch)
                    native(name, iso, cpu, [m for m in (ROOT / "tools/tests" / expected).read_text().splitlines() if m and not m.startswith("#")])
            if args.section in ("all", "lifecycle"):
                host = ["cargo", "test", "--locked", "--offline", "--target", "x86_64-unknown-linux-gnu"]
                check("sunlightd-runtime-host", host + ["-p", "sunlightd", "--bin", "sunlightd", "--features", "dependency-test"])
                users("dependency-users", ["sunlightd", "sunlight-startup-fixture"], "sunlightd/dependency-test,sunlight-startup-fixture/dependency-test")
                iso = image("dependency", "dependency_test", scratch)
                if iso:
                    check("d1-d2-d3", ["python3", "tools/sunlightd_d1_gate.py", "--d2", "--d3", "--iso", str(iso), "--evidence", str(out / "d1-d2-d3"), "--cpus", "1", "2", "4"], timeout=1200)
                users("recovery-users", ["sunlight-init", "sunlightd", "sunlight-startup-fixture"], "sunlight-init/recovery-test,sunlightd/recovery-test,sunlight-startup-fixture/recovery-test")
                iso = image("recovery", "recovery_test", scratch, {"D36R_SCENARIO": "after-ready"})
                if iso:
                    check("recovery-blocked", ["python3", "tools/sunlightd_d1_gate.py", "--d2", "--d3", "--d36r", "--d36r-scenario", "after-ready", "--iso", str(iso), "--evidence", str(out / "recovery"), "--cpus", "1", "2", "4"], timeout=1200)
                users("restore-ordinary-users")
            if args.section in ("all", "network"):
                iso = image("resident", "fastlane_f1_audit", scratch)
                native("resident-12cpu", iso, 12, ["[FASTLANE-F1] prerequisite lifetime gate: OK (channel remains BLOCKED)", "[SunlightOS] sunlightd OK"])
                if iso:
                    for cpu in (1, 2, 4, 12):
                        check(f"tcp-tls-{cpu}cpu", ["python3", "tools/network_tls_gate.py", "--iso", str(iso), "--cpus", str(cpu), "--timeout", "240", "--evidence", str(out / f"tcp-tls-{cpu}cpu")], timeout=600)
    finally:
        summary["source_sha256_after"] = sources()
        summary["source_hashes_unchanged"] = summary["source_sha256_after"] == summary["source_sha256"]
        save()
        print("Evidence:", out.relative_to(ROOT), flush=True)
    if not summary["source_hashes_unchanged"] or any(c["exit_code"] for c in summary["checks"]) or any(r["classification"] != "PASS" for r in summary["native"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
