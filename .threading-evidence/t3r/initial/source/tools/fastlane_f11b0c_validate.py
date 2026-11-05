#!/usr/bin/env python3
"""Production grant evidence; missing native races keep F1.1B-0 BLOCKED.

Every invocation retains an independent attempt. --native builds the real UAC
and VFS, plus an opt-in unprivileged client; production roles are never forged.
No result in this runner upgrades Native Doorbell Gate B.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "target/fastlane-f11b0c-evidence"
SERVICE_FLAGS = "-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone"
SOURCES = [
    "ipc/src/vfs_grants.rs", "ipc/src/lib.rs", "kernel/Cargo.toml",
    "kernel/src/capability/mod.rs", "kernel/src/capability/lifecycle.rs",
    "kernel/src/capability/delivery.rs", "kernel/src/capability/vfs_grants.rs",
    "kernel/src/capability/lifecycle_tests.rs", "kernel/src/capability/production_tests.rs",
    "kernel/src/capability/production_gate.rs", "kernel/src/capability/lifecycle_host.rs",
    "kernel/src/capability/tests.rs", "kernel/src/entropy.rs", "kernel/src/entropy/conditioner.rs",
    "kernel/src/process/mod.rs", "kernel/src/process/spawn.rs", "kernel/src/process/exec_authority.rs",
    "kernel/src/process/foreground.rs", "kernel/src/process/mm2b_state.rs", "kernel/src/ipc/mod.rs",
    "kernel/src/sched/mod.rs",
    "kernel/src/arch/x86_64/syscall.rs", "kernel/src/arch/x86_64/interrupts.rs", "kernel/src/main.rs",
    "services/sunlight-uac/Cargo.toml", "services/sunlight-uac/src/delegation.rs",
    "services/sunlight-uac/src/lib.rs",
    "services/sunlight-uac/src/bin/uac_service.rs", "services/sunlight-uac/src/bin/account_service/mod.rs",
    "services/sunlight-uac/src/bin/grant_probe.rs", "services/vfs_server/src/main.rs",
    "tools/fastlane_f11b0c_validate.py",
    "tools/test_fastlane_f11b0c.py",
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def assess_native(raw, cpus):
    failures = []
    match = re.search(r"\[GRANT-C\] COMPLETE PASS cpus=(\d+) clients=(\d+) committed=(\d+) delivered=(\d+) consumed=(\d+) owner_retired=(\d+) delivery_retired=(\d+) exec_retired=(\d+) inventory=(\d+) pending=(\d+) issuer_refs=(\d+) recipient_refs=(\d+) remote_race=NOT_FORCED", raw)
    expected = [cpus, cpus, cpus * 11, cpus * 9, cpus * 4, cpus * 3, cpus * 2, cpus * 2, 0, 0, 0, 0]
    if match is None or list(map(int, match.groups())) != expected:
        failures.append("missing or incorrect authoritative final inventory")
    commits = re.findall(r"\[GRANT-C\] commit issuer=(\d+) issuer_inc=(\d+) role=uac recipient=(\d+) incarnation=(\d+) vm=(\d+) request=(\d+) batch=(\d+) ordinal=(\d+) cpu=(\d+)", raw)
    identities = {}
    for row in commits:
        issuer, issuer_inc, recipient, incarnation, vm, request, batch, ordinal, cpu = map(int, row)
        key = (recipient, incarnation, vm)
        identities.setdefault(key, []).append(ordinal)
        if min(issuer, issuer_inc, incarnation, vm, request) <= 0 or batch != (1 if ordinal == 5 else 2) or cpu >= cpus:
            failures.append("invalid authenticated identity or commit")
    if len(identities) != cpus or any(sorted(v) != [1, 2, 3, 4, 5, 6] for v in identities.values()):
        failures.append("incomplete recipient commit sequences")
    unprivileged = {int(pid) for pid in re.findall(r"\[GRANT-C\] client pid=(\d+) uid=1000 gid=1000 phase=invalid-issuer-and-proof PASS", raw)}
    if unprivileged != {key[0] for key in identities}:
        failures.append("missing non-root requester identity assertion")
    if len(re.findall(r"\[GRANT-C\] exec recipient=.*old_token=denied inventory=0", raw)) != cpus:
        failures.append("missing real exec/VM withdrawal")
    if len(re.findall(r"\[GRANT-C\] reaped recipient=\d+ incarnation=\d+ exit=0 cpu=\d+", raw)) != cpus:
        failures.append("missing real reaper transition")
    owner_retired = re.findall(r"\[GRANT-C\] owner-cancel recipient=(\d+) retired=(\d+) inventory=(\d+) cpu=(\d+)", raw)
    if sum(int(row[1]) for row in owner_retired) != cpus * 3:
        failures.append("incomplete recipient cancellation accounting")
    if any(marker in raw for marker in ("PANIC", "panicked at", "[GRANT-C] client FAIL", "[FAULT]", "[D36R-TEST]", "[D3-TEST]")):
        failures.append("guest failure")
    metadata = re.search(r"\[GRANT-C\] metadata vfs_active=(\d+) vfs_capacity=(\d+) vfs_entry_bytes=(\d+) auth_active=(\d+) auth_capacity=(\d+) allocator_drift=UNMEASURED", raw)
    if metadata is None:
        failures.append("missing table metadata observation")
    return {"classification": "PASS" if not failures else "FAIL", "failures": failures,
            "recipient_assertions": [{"pid": k[0], "incarnation": k[1], "vm_generation": k[2], "uid": 1000 if k[0] in unprivileged else None, "commits": v}
                                     for k, v in identities.items()],
            "counts": dict(zip(["cpus", "clients", "committed", "delivered", "consumed", "owner_retired", "delivery_retired", "exec_retired", "active", "pending", "issuer_refs", "recipient_refs"], map(int, match.groups()))) if match else None,
            "probes": {"N1_real_base_and_exact_delegation": "PASS" if not failures else "FAIL",
                       "N2_recipient_exit_before_delivery": "NOT FORCED",
                       "N3_recipient_exec_after_delivery": "PASS" if not failures else "FAIL",
                       "N3_remote_exec_during_delivery": "NOT FORCED",
                       "N4_issuer_lifecycle_at_commit": "NOT FORCED",
                       "N5_actual_pid_reuse": "NOT FORCED",
                       "N6_real_reply_cancellation": "PASS" if not failures else "FAIL",
                       "N7_service_restart": "NOT FORCED"},
            "table_metadata": dict(zip(["vfs_active", "vfs_capacity", "vfs_entry_bytes", "auth_active", "auth_capacity"], map(int, metadata.groups()))) if metadata else None,
            "allocator_drift": None, "remote_lifecycle_race": "NOT FORCED"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true")
    parser.add_argument("--checks", action="store_true")
    parser.add_argument("--cpus", type=int, nargs="+", default=[1, 2, 4, 12])
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--iso", type=Path, help="use a retained production-grant fixture image")
    args = parser.parse_args()
    if any(c not in (1, 2, 4, 12) for c in args.cpus) or args.timeout < 1:
        parser.error("use 1/2/4/12 CPUs and positive timeout")
    now = datetime.now(timezone.utc)
    evidence = BASE / now.strftime("%Y%m%dT%H%M%S.%fZ")
    evidence.mkdir(parents=True)
    summary = {
        "schema_version": 1, "utc": now.isoformat(),
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "build_environment": {key: os.environ.get(key) for key in ("CARGO_INCREMENTAL", "CARGO_TARGET_DIR", "RUSTFLAGS", "SUNLIGHT_CAP_EXEC_PROBE")},
        "raw_directory": str(evidence.relative_to(ROOT)), "checks": [], "native": [],
        "f11b_0": "BLOCKED", "native_doorbell_gate_b": "PARTIAL", "production_fastlane_gate_c": "BLOCKED",
        "production_entry_points": ["uac_service::handle_auth_password", "sunlight_uac::delegation::handle",
                                    "sys_vfs_grant", "vfs_server::handle_request::AUTHORIZE_EXEC",
                                    "Scheduler::delegate_vfs_grants", "Scheduler::consume_vfs_grant",
                                    "ipc::deliver_reply_to_target", "exec_into_process"],
        "required_missing_evidence": ["retired observable token non-reassignment contract",
                                      "Account active-session withdrawal serialized with commit/publication",
                                      "remote issuer exec/exit before commit", "remote recipient exit/exec during delivery",
                                      "native actual PID reuse", "native service restart with outstanding grants",
                                      "complete native Account session-withdrawal matrix", "native metadata allocation accounting"],
        "physical_frame_drift": None, "metadata_allocation_drift": None,
        "note": "VFS authorization operation does not change legacy file I/O or enforce execution/spawn policy",
    }

    def save():
        for path in (evidence / "validation-summary.json", BASE / "validation-summary.json"):
            path.write_text(json.dumps(summary, indent=2) + "\n")

    def check(name, command, timeout=900, env=None):
        started = time.monotonic()
        with (evidence / (name + ".log")).open("w") as log:
            log.write("command: " + json.dumps(command) + "\n")
            if env: log.write("environment: " + json.dumps(env) + "\n")
            log.flush()
            try:
                code = subprocess.run(command, cwd=ROOT, env={**os.environ, **(env or {})}, stdout=log, stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        summary["checks"].append({"name": name, "command": command, "environment": env or {}, "exit_code": code,
                                  "classification": "PASS" if code == 0 else "FAIL", "seconds": time.monotonic()-started,
                                  "log": str((evidence / (name + ".log")).relative_to(ROOT))})
        print(name, "PASS" if code == 0 else "FAIL", flush=True)
        save()
        return code == 0

    try:
        summary["source_sha256"] = {p: digest(ROOT / p) for p in SOURCES}
        for p in SOURCES:
            dest = evidence / "source" / p
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / p, dest)
        (evidence / "working-tree.patch").write_bytes(subprocess.check_output(["git", "diff", "HEAD"], cwd=ROOT))
        summary["rustc"] = subprocess.check_output(["rustc", "-vV"], text=True)
        if args.checks:
            host = ["cargo", "test", "--locked", "--offline", "--target", "x86_64-unknown-linux-gnu"]
            check("kernel-host", host + ["-p", "sunlight-kernel", "--lib", "--", "--test-threads=1"])
            check("services-host", host + sum((["-p", p] for p in ["sunlight-ipc", "sunlight-uac", "sunlight-fs", "sunlightd", "sunlight-audiod", "sunlight-elf", "sunlight-fastlane-ring", "sunlight-fastlane-doorbell"]), []) + ["--lib"])
            check("scheduler-host", ["python3", "tools/test_scheduler_host.py"])
            check("evidence-parser", ["python3", "-m", "unittest", "discover", "-s", "tools", "-p", "test_fastlane_f11b0c.py"])
            check("kernel-target", ["cargo", "check", "--locked", "--offline", "-p", "sunlight-kernel", "--bin", "sunlight-kernel"])
        if args.native:
            summary["qemu"] = subprocess.check_output(["qemu-system-x86_64", "--version"], text=True).splitlines()[0]
            iso = args.iso
            if iso is None:
                services = check("production-services", ["cargo", "build", "--locked", "--offline", "-p", "sunlight-uac", "-p", "sunlight-vfs-server", "--release", "--features", "sunlight-uac/production-grant-test"], env={"RUSTFLAGS": SERVICE_FLAGS})
                built = services and check("production-kernel", ["cargo", "build", "--locked", "--offline", "-p", "sunlight-kernel", "--bin", "sunlight-kernel", "--features", "production_grant_test"])
                if built:
                    elf = evidence / "production.elf"
                    iso = evidence / "production.iso"
                    shutil.copy2(ROOT / "target/x86_64-unknown-none/debug/sunlight-kernel", elf)
                    summary["elf_sha256"] = digest(elf)
                    if not check("production-iso", ["bash", "tools/make_hybrid_iso.sh", str(elf), str(iso), str(ROOT / "target/limine"), str(ROOT)]):
                        iso = None
            if iso:
                iso = iso.resolve()
                summary["iso"] = str(iso)
                summary["iso_sha256"] = digest(iso)
                if args.iso is None:
                    summary["embedded_services_sha256"] = {p: digest(ROOT / "target/x86_64-unknown-none/release" / p) for p in ["uac_service", "sunlight-vfs-server", "grant-probe", "sunlight-init", "sunlightd"]}
                else:
                    summary["retained_image_provenance"] = "caller-supplied; current service artifacts do not establish embedded identity"
                for cpus in args.cpus:
                    serial = evidence / f"production-{cpus}cpu.serial.log"
                    serial.touch()
                    command = ["qemu-system-x86_64", "-accel", "tcg,thread=multi", "-cpu", "max", "-smp", str(cpus), "-m", "1024", "-cdrom", str(iso), "-serial", f"file:{serial}", "-display", "none", "-nic", "none", "-no-reboot", "-no-shutdown", "-device", "virtio-rng-pci,disable-modern=on", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]
                    started = time.monotonic()
                    with (evidence / f"production-{cpus}cpu.qemu.log").open("w") as log:
                        proc = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
                        try:
                            while proc.poll() is None and time.monotonic()-started < args.timeout:
                                raw = serial.read_text(errors="replace")
                                if "[GRANT-C] COMPLETE PASS" in raw or "[GRANT-C] client FAIL" in raw or "PANIC" in raw: break
                                time.sleep(.25)
                        finally:
                            if proc.poll() is None:
                                proc.terminate()
                                try: proc.wait(timeout=5)
                                except subprocess.TimeoutExpired: proc.kill()
                            code = proc.wait()
                    result = assess_native(serial.read_text(errors="replace"), cpus)
                    result.update({"cpus": cpus, "command": command, "exit_code": code, "seconds": time.monotonic()-started, "serial": str(serial.relative_to(ROOT))})
                    summary["native"].append(result)
                    save()
                    print(f"production-{cpus}cpu {result['classification']}", flush=True)
        summary["source_hashes_unchanged"] = summary["source_sha256"] == {p: digest(ROOT / p) for p in SOURCES}
        if not summary["source_hashes_unchanged"]:
            summary["checks"].append({"name": "source-stability", "classification": "FAIL",
                                      "reason": "sources changed during validation"})
    finally:
        save()
        print("Evidence:", evidence.relative_to(ROOT), flush=True)
    if any(x["classification"] == "FAIL" for x in summary["checks"] + summary["native"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
