#!/usr/bin/env python3
"""Retain capability-issuance security evidence; restricted grants stay deferred.

Each invocation keeps a separate raw run directory, including failed attempts.
The top-level validation-summary.json points to the latest complete attempt.
TCG tests are functional evidence, not production timing/throughput estimates.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "target/fastlane-f11pr-evidence"
SOURCES = [
    "kernel/src/capability/lifecycle.rs", "kernel/src/capability/lifecycle_tests.rs",
    "kernel/src/capability/lifecycle_host.rs", "kernel/src/process/spawn.rs",
    "kernel/src/capability/mod.rs", "kernel/src/capability/tests.rs",
    "kernel/src/capability/gate.rs", "kernel/src/entropy.rs",
    "kernel/src/entropy/conditioner.rs", "kernel/src/mm2a_host.rs",
    "kernel/src/memory/shared.rs", "kernel/src/memory/shared_authority_gate.rs",
    "kernel/src/memory/shared_lifetime_gate.rs", "kernel/src/memory/tlb.rs",
    "kernel/src/arch/x86_64/syscall.rs", "kernel/src/arch/x86_64/interrupts.rs",
    "kernel/src/main.rs", "kernel/Cargo.toml", "kernel/build.rs",
    "kernel/src/delegated_caller.rs", "kernel/src/sched/mod.rs",
    "kernel/src/ipc/mod.rs", "kernel/src/memory/mod.rs", "kernel/src/memory/security.rs",
    "tools/helios-probes/capability-probe.S", "tools/fastlane_f11pr_validate.py",
    "tools/fastlane_f11p_desktop_check.py",
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true")
    parser.add_argument("--regressions", action="store_true")
    parser.add_argument("--cpus", type=int, nargs="+", default=[1, 2, 4, 12])
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    if any(c not in (1, 2, 4, 12) for c in args.cpus) or args.timeout < 1:
        parser.error("use CPUs 1/2/4/12 and positive timeout")
    now = datetime.now(timezone.utc)
    evidence = BASE / now.strftime("%Y%m%dT%H%M%S.%fZ")
    evidence.mkdir(parents=True)
    summary = {
        "schema_version": 1, "utc": now.isoformat(), "f11pr_complete": False, "f11b_0_unblocked": False,
        "status": "implemented_but_validation_incomplete",
        "f11_restricted_grants_complete": False,
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "raw_directory": str(evidence.relative_to(ROOT)),
        "token_payload_bits": 62, "random_candidate_retry_limit": 16,
        "active_token_limit": 8192, "endpoint_limit": 4096,
        "checks": [], "native": [], "limitations": [],
        "not_implemented": ["recipient-bound restricted memory grants", "active per-grant PTE revocation",
                            "ring3 RO/post-revoke faults", "SPSC ring", "Doorbell"],
    }
    failures = []

    def check(name, command, timeout=900, media=False):
        log = evidence / f"{name}.log"
        started = time.monotonic()
        with log.open("w") as output:
            output.write("command: " + json.dumps(command) + "\n")
            output.flush()
            try:
                code = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT,
                                      timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        item = {"name": name, "command": command, "exit_code": code,
                "wall_seconds": time.monotonic() - started, "log": str(log.relative_to(ROOT))}
        summary["checks"].append(item)
        if code and media:
            text = log.read_text(errors="replace")
            fixture = ROOT / "docs/songs/onaldin_music-catch-the-sunlight-333176.wav"
            if (not fixture.exists() and "36 passed; 1 failed" in text
                    and "mp3_converted_repository_wav_is_valid_but_exceeds_loader_limit ... FAILED" in text
                    and "No such file or directory" in text):
                item["known_unrelated_failure"] = "missing repository WAV fixture"
                summary["limitations"].append(item["known_unrelated_failure"])
                print(f"{name}: KNOWN FAILURE (raw evidence retained)", flush=True)
                return True
        print(f"{name}: {'PASS' if code == 0 else 'FAIL'}", flush=True)
        if code:
            failures.append(name)
        return code == 0

    def native(name, iso, cpus, markers, no_entropy=False):
        serial = evidence / f"{name}.serial.log"
        serial.write_text("")
        command = ["qemu-system-x86_64", "-accel", "tcg,thread=multi", "-cpu",
                   "max,-rdrand,-rdseed" if no_entropy else "max", "-smp", str(cpus),
                   "-m", "1024", "-cdrom", str(iso), "-serial", f"file:{serial}",
                   "-display", "none", "-nic", "none", "-no-reboot", "-no-shutdown",
                   "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]
        if not no_entropy:
            command += ["-device", "virtio-rng-pci,disable-modern=on"]
        started = time.monotonic()
        with (evidence / f"{name}.qemu.log").open("w") as log:
            proc = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = started + args.timeout
                while time.monotonic() < deadline:
                    output = serial.read_text(errors="replace")
                    if all(m in output for m in markers) or proc.poll() is not None:
                        break
                    if "PANIC" in output or "UNEXPECTED" in output:
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
        output = serial.read_text(errors="replace")
        missing = [m for m in markers if m not in output]
        passed = not missing and "PANIC" not in output and "UNEXPECTED" not in output
        if no_entropy:
            passed &= "[CAP-P] native begin" not in output and "[PROC]" not in output
        lines = [line for line in output.splitlines() if line.startswith("[CAP-P]")]
        item = {"name": name, "cpus": cpus, "pass": passed, "command": command,
                "exit_code": code, "wall_seconds": time.monotonic() - started,
                "serial": str(serial.relative_to(ROOT)), "missing": missing, "diagnostics": lines}
        summary["native"].append(item)
        if not passed:
            failures.append(name)
        print(f"{name}: {'PASS' if passed else 'FAIL'}", flush=True)

    def image(name, feature):
        if not check(f"{name}-build", ["cargo", "build", "--locked", "--offline", "-p",
                     "sunlight-kernel", "--bin", "sunlight-kernel", "--features", feature]):
            return None
        elf, iso = evidence / f"{name}.elf", evidence / f"{name}.iso"
        shutil.copy2(ROOT / "target/x86_64-unknown-none/debug/sunlight-kernel", elf)
        if not check(f"{name}-iso", ["bash", "tools/make_hybrid_iso.sh", str(elf), str(iso),
                                   str(ROOT / "target/limine"), str(ROOT)]):
            return None
        summary.setdefault("images", {})[name] = {
            "features": [feature], "elf_sha256": digest(elf), "iso_sha256": digest(iso),
            "iso": str(iso.relative_to(ROOT)),
        }
        return iso

    try:
        summary["source_sha256"] = {s: digest(ROOT / s) for s in SOURCES}
        (evidence / "working-tree.patch").write_bytes(subprocess.check_output(["git", "diff", "HEAD"], cwd=ROOT))
        summary["rustc"] = subprocess.check_output(["rustc", "--version"], text=True).strip()
        host = ["cargo", "test", "--locked", "--offline", "--target", "x86_64-unknown-linux-gnu"]
        check("kernel-host", host + ["-p", "sunlight-kernel", "--lib", "--", "--test-threads=1"])
        check("ipc-services-host", host + ["-p", "sunlight-ipc", "-p", "sunlight-audiod", "-p", "sunlightd", "--lib"])
        check("media-host", host + ["-p", "sunlight-media", "--lib"], media=True)
        check("scheduler-host", ["python3", "tools/test_scheduler_host.py"])
        check("default-check", ["cargo", "check", "--locked", "--offline", "-p", "sunlight-kernel", "--bin", "sunlight-kernel"])
        if args.native:
            summary["qemu"] = subprocess.check_output(["qemu-system-x86_64", "--version"], text=True).splitlines()[0]
            iso = image("authority", "cap_issuance_test")
            if iso:
                native("entropy-unavailable", iso, 1, ["[CAP] bootstrap BLOCKED: secure entropy unavailable; no user authority published"], no_entropy=True)
                for cpus in args.cpus:
                    native(f"authority-{cpus}cpu", iso, cpus,
                           [f"[CAP-P] native PASS cpus={cpus} ",
                            "[CAP-P] SHM prediction regression: 1984 denied; 32 delegated legacy maps; zero frame drift",
                            "[CAP-P] local fault injection: entropy/reservation/SHM rollback/collision/retry bound OK",
                            f"[CAP-PR] lifecycle PASS auth=16 account=16 post_reap_records=0 late_auth=denied late_account=denied cpus={cpus}"])
            resident = image("resident", "fastlane_f1_audit")
            if resident:
                markers = [line for name in ("phase_sec.expected", "phase_shm.expected")
                           for line in (ROOT / "tools/tests" / name).read_text().splitlines()
                           if line and not line.startswith("#")]
                markers += ["[FASTLANE-F1] capability issuance regression: adjacent token prediction REJECTED",
                            "[FASTLANE-F1] prerequisite lifetime gate: OK (channel remains BLOCKED)",
                            "[SunlightOS] Phase 2.6 OK", "[SunlightOS] sunlightd OK"]
                for cpus in args.cpus:
                    native(f"resident-{cpus}cpu", resident, cpus, markers + [f"[FASTLANE-F1] remote SHM release: cpus={cpus} acknowledgements={cpus - 1} OK"])
        if args.regressions:
            check("scheduler-native", ["python3", "tools/test_scheduler_smp.py", "--cpus", *map(str, args.cpus),
                                       "--evidence-dir", str(evidence / "scheduler")], timeout=1800)
            check("recovery-native", ["bash", "tools/test_sunlightd_d36r.sh", "--cpus", "1", "2", "4"], timeout=1800)
            recovery = ROOT / "target/d36r-evidence/after-ready"
            if recovery.exists():
                shutil.copytree(recovery, evidence / "recovery", dirs_exist_ok=True)
        check("diff-check", ["git", "diff", "--check"])
        final_hashes = {s: digest(ROOT / s) for s in SOURCES}
        summary["source_hashes_verified_after_validation"] = final_hashes == summary["source_sha256"]
        if not summary["source_hashes_verified_after_validation"]:
            failures.append("sources changed during validation")
        summary["failures"] = failures
        summary["status"] = "implemented_and_targeted_validation_passed" if not failures else "validation_failed"
        # Network/TLS and full graphical startup need separately retained gates;
        # never infer them merely from resident serial markers.
        summary["acceptance_pending"] = ["native deterministic grant/cleanup boundary interleavings",
            "native exec/PID reuse and grant failure matrix", "native Wise Owl lifecycle races",
            "TCP/TLS compatibility", "resident graphical startup", "matched lifecycle lock timing"]
        summary["baseline_counterexamples"] = "target/fastlane-f11pr-evidence/before-counterexamples.log"
        summary["reservation_counts"] = None
        summary["lifecycle_latency_percentiles"] = None
    finally:
        summary["failures"] = failures
        path = evidence / "validation-summary.json"
        path.write_text(json.dumps(summary, indent=2) + "\n")
        (BASE / "validation-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(f"Evidence: {evidence.relative_to(ROOT)}", flush=True)
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
