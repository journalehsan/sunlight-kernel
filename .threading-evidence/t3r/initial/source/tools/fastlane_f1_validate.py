#!/usr/bin/env python3
"""Retain F1 prerequisite evidence. Success here does not mean FastLane exists.

--native builds an opt-in audit image and tests real VM/TLB mechanisms, then
continues to resident startup. It does not run a ring-3 producer/consumer pair.
All commands, source/image hashes, logs and unavailable measurements are retained.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "target/fastlane-f1-evidence"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true")
    parser.add_argument("--cpus", nargs="+", type=int, default=[1, 2, 4])
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    if any(c not in (1, 2, 4, 12) for c in args.cpus) or args.timeout < 1:
        parser.error("use CPUs 1, 2, 4 or 12 and a positive timeout")
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    summary = {
        "schema_version": 1,
        "utc": datetime.now(timezone.utc).isoformat(),
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "f1_complete": False,
        "status": "blocked",
        "blocker": "F1B: no recipient-bound mapping rights or live channel revocation; ordinary issuance repaired separately by F1.1P",
        "subsystems": {
            "F1A_audit": "implemented_and_validated",
            "backing_retirement": "implemented_but_not_fully_validated",
            "F1B_channel_grants_and_live_revocation": "blocked",
            "F1C_ring": "deferred",
            "F1D_doorbell": "deferred",
            "native_fastlane_pair": "blocked",
            "comparative_benchmarks": "deferred",
        },
        "checks": [],
        "native_vm_tests": [],
        "performance": {name: {
            "status": "not_measured" if name == "A_existing_IPC" else "unavailable",
            "samples": 0, "reason": "Stopped at F1B isolation prerequisite",
            "messages_per_second": None, "payload_bytes_per_second": None,
            "latency_ns": None, "context_switches_per_message": None,
            "kernel_crossings_per_message": None, "cpu_busy_ns": None,
        } for name in ("A_existing_IPC", "B_persistent_SHM_IPC", "C_FastLane_Doorbell")},
        "not_tested": [
            "ring memory ordering", "lost-wakeup races", "ring-3 FastLane peers",
            "channel capability generation reuse", "native FastLane SMP stress",
            "audio playback and recording", "TCP/TLS", "interactive graphical startup",
            "FastLane quotas and in-flight channel revocation",
        ],
    }

    def check(name, command, timeout=600, missing_media_fixture=False):
        log_path = EVIDENCE / f"{name}.log"
        started = time.monotonic()
        with log_path.open("w") as log:
            log.write("command: " + json.dumps(command) + "\n")
            log.flush()
            try:
                result = subprocess.run(command, cwd=ROOT, stdout=log,
                                        stderr=subprocess.STDOUT, timeout=timeout)
                code = result.returncode
            except subprocess.TimeoutExpired:
                code = 124
        summary["checks"].append({"name": name, "command": command, "exit_code": code,
                                  "wall_seconds": time.monotonic() - started,
                                  "log": str(log_path.relative_to(ROOT))})
        if code and missing_media_fixture:
            output = log_path.read_text(errors="replace")
            fixture = ROOT / "docs/songs/onaldin_music-catch-the-sunlight-333176.wav"
            if (not fixture.exists()
                    and "test result: FAILED. 36 passed; 1 failed;" in output
                    and "decoder::tests::mp3_converted_repository_wav_is_valid_but_exceeds_loader_limit ... FAILED" in output
                    and "No such file or directory" in output):
                summary["checks"][-1]["unrelated_limitation"] = f"Missing untracked fixture {fixture}"
                print(f"{name}: FAIL (retained unrelated missing WAV fixture)", flush=True)
                return
        print(f"{name}: {'PASS' if code == 0 else 'FAIL'}", flush=True)
        if code:
            raise RuntimeError(f"{name} failed; see {log_path}")

    try:
        (EVIDENCE / "working-tree.patch").write_bytes(
            subprocess.check_output(["git", "diff", "HEAD"], cwd=ROOT))
        sources = ["kernel/src/memory/shared.rs", "kernel/src/memory/shared_lifetime.rs",
                   "kernel/src/memory/shared_lifetime_gate.rs", "kernel/src/memory/tlb.rs",
                   "kernel/src/capability/mod.rs", "kernel/src/main.rs", "kernel/Cargo.toml",
                   "kernel/src/mm2a_host.rs", "kernel/src/memory/mod.rs",
                   "tools/fastlane_f1_validate.py"]
        summary["source_sha256"] = {s: digest(ROOT / s) for s in sources}
        summary["rustc"] = subprocess.check_output(["rustc", "--version"], cwd=ROOT, text=True).strip()
        summary["host"] = dict(zip(("system", "node", "release", "version", "machine"), os.uname()))
        check("lifetime-host-build", ["rustc", "--edition=2021", "--test",
              "kernel/src/memory/shared_lifetime.rs", "-o", str(EVIDENCE / "lifetime-host")])
        check("lifetime-host", [str(EVIDENCE / "lifetime-host")])
        check("host-regressions", ["cargo", "test", "--locked", "--offline", "-p", "sunlight-ipc",
              "-p", "sunlight-audiod", "--lib", "--target",
              "x86_64-unknown-linux-gnu"])
        check("media-host", ["cargo", "test", "--locked", "--offline", "-p", "sunlight-media",
              "--lib", "--target", "x86_64-unknown-linux-gnu"], missing_media_fixture=True)
        check("kernel-host", ["cargo", "test", "--locked", "--offline", "-p", "sunlight-kernel",
              "--lib", "--target", "x86_64-unknown-linux-gnu", "--", "--test-threads=1"])
        check("scheduler-host", ["python3", "tools/test_scheduler_host.py"])
        # The scheduler runner retains its own raw output; copy it into this run.
        (EVIDENCE / "scheduler-focused-host.log").write_bytes(
            (ROOT / "target/scheduler-b-evidence/focused-host.log").read_bytes())
        check("kernel-default-check", ["cargo", "check", "--locked", "--offline", "-p",
              "sunlight-kernel", "--bin", "sunlight-kernel"])
        if args.native:
            check("kernel-audit-build", ["cargo", "build", "--locked", "--offline", "-p",
                  "sunlight-kernel", "--bin", "sunlight-kernel", "--features", "fastlane_f1_audit"])
            elf = EVIDENCE / "prerequisites.elf"
            elf.write_bytes((ROOT / "target/x86_64-unknown-none/debug/sunlight-kernel").read_bytes())
            iso = EVIDENCE / "prerequisites.iso"
            check("audit-iso", ["bash", "tools/make_hybrid_iso.sh", str(elf), str(iso),
                  str(ROOT / "target/limine"), str(ROOT)])
            summary["image"] = {"elf_sha256": digest(elf), "iso_sha256": digest(iso),
                                "features": ["fastlane_f1_audit"], "accelerator": "tcg"}
            summary["qemu"] = subprocess.check_output(
                ["qemu-system-x86_64", "--version"], text=True).splitlines()[0]
            required = [line for name in ("phase_sec.expected", "phase_shm.expected")
                        for line in (ROOT / "tools/tests" / name).read_text().splitlines()
                        if line and not line.startswith("#")]
            required += [
                "[FASTLANE-F1] capability issuance regression: adjacent token prediction REJECTED",
                "[FASTLANE-F1] isolation prerequisite: BLOCKED (legacy bearer RW/NX)",
                "[FASTLANE-F1] retired views, failed-unmap preflight, double free, 32 lifecycles: OK",
                "[FASTLANE-F1] prerequisite lifetime gate: OK (channel remains BLOCKED)",
                "[SunlightOS] Phase 2.6 OK",
                "[SunlightOS] sunlightd OK",
            ]
            for cpus in args.cpus:
                serial = EVIDENCE / f"{cpus}cpu.serial.log"
                serial.write_text("")
                command = ["qemu-system-x86_64", "-accel", "tcg,thread=multi", "-cpu", "max",
                           "-smp", str(cpus), "-m", "1024", "-cdrom", str(iso), "-serial",
                           f"file:{serial}", "-display", "none", "-nic", "none",
                           "-device", "virtio-rng-pci,disable-modern=on", "-no-reboot", "-no-shutdown"]
                deadline = time.monotonic() + args.timeout
                markers = required + [f"[FASTLANE-F1] remote SHM release: cpus={cpus} acknowledgements={cpus - 1} OK"]
                with (EVIDENCE / f"{cpus}cpu.qemu.log").open("w") as log:
                    proc = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
                    try:
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
                summary["native_vm_tests"].append({"cpus": cpus, "pass": passed,
                    "command": command, "qemu_exit_code": code, "missing": missing,
                    "scope": "kernel VM/TLB prerequisite plus resident boot; not FastLane IPC",
                    "lifecycle_iterations": 32, "unexplained_frame_drift": 0 if passed else None,
                    "serial": str(serial.relative_to(ROOT))})
                print(f"{cpus} CPU prerequisite/resident gate: {'PASS' if passed else 'FAIL'}", flush=True)
                if not passed:
                    raise RuntimeError(f"native prerequisite gate failed on {cpus} CPU(s)")
            summary["subsystems"]["backing_retirement"] = "implemented_and_validated"
        check("diff-check", ["git", "diff", "--check"])
        print("F1 remains BLOCKED at recipient-specific mapping authority and active revocation; no transport benchmark.", flush=True)
    finally:
        (EVIDENCE / "validation-summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
