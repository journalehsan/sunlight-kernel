#!/usr/bin/env python3
"""F0F evidence: real production delegation, allocator contract and IPC census.

No bare marker stream certifies an image or closes a global userspace snapshot.
Each invocation preserves its own manifest; consolidation never rewrites one.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

from fastlane_f11b0c_validate import SERVICE_FLAGS, assess_native
from fastlane_f11b0d_validate import assess_races
from fastlane_f11b0e3_validate import BUILD, ENV, ROOT, digest, sources

BASE = ROOT / "target/fastlane-f11b0f-evidence"
GATES = {"A": "VERIFIED NATIVE", "B": "PARTIAL", "session_authority": "PARTIAL",
         "token_history": "PARTIAL", "F1.1B-0": "BLOCKED", "C": "BLOCKED"}


def fields(line):
    return {k: int(v) if v.isdigit() else v for k, v in re.findall(r"(\w+)=(\S+)", line)}


def assess_f(raw, cpus, identity=None):
    errors = []
    published, reaped, snapshots, rows, lifecycle = [], [], [], [], []
    for line in raw.splitlines():
        if "[GRANT-F] pid-published " in line:
            published.append(fields(line))
        elif "[GRANT-F] pid-reaped " in line:
            reaped.append(fields(line))
        elif "[GRANT-F] row " in line:
            rows.append(fields(line))
        elif "[GRANT-F] census " in line:
            census = fields(line)
            census["rows"] = rows
            rows = []
            snapshots.append(census)
        elif "[GRANT-F] lifecycle " in line:
            lifecycle.append({**fields(line), "position": raw.find(line)})
    try:
        if not identity or not all(identity.get(k) is True for k in
                ("source_unchanged", "iso_matches", "elf_matches", "embedded_match")):
            errors.append("missing verified source/image/embedded identity")
        if len(published) != 8 or len(reaped) != 8:
            errors.append("missing or duplicate real spawn/reap observations")
        if len({r["pid"] for r in published}) != 8 or len({r["incarnation"] for r in published}) != 8:
            errors.append("PID/incarnation publication did not advance")
        if [r["pid"] for r in published] != sorted(r["pid"] for r in published):
            errors.append("PID order invalid")
        for row in published:
            if row["parent"] != 0xF4F0 or row["roles"] != 0 or min(row[k] for k in ("pid", "incarnation", "vm")) <= 0:
                errors.append("invalid real process identity or unauthorized role")
            matching = [r for r in reaped if all(r[k] == row[k] for k in ("pid", "incarnation", "vm"))]
            if len(matching) != 1 or any(matching[0][k] != 0 for k in ("pending", "reply", "deferred", "stack")):
                errors.append("reap identity/cleanup mismatch")
            if raw.find("[GRANT-F] pid-published parent=62704 pid=" + str(row["pid"]) + " ") >= raw.find("[GRANT-F] pid-reaped pid=" + str(row["pid"]) + " "):
                errors.append("reap before publication")
        if not snapshots or snapshots[0]["phase"] != "start" or snapshots[-1]["phase"] != "final":
            errors.append("missing census boundaries")
        if sum(s["phase"] == "start" for s in snapshots) != 1 or sum(s["phase"] == "final" for s in snapshots) != 1:
            errors.append("duplicate census boundary")
        if any(s["phase"] not in ("start", "operation", "final") for s in snapshots):
            errors.append("invented census phase")
        if not any(s["phase"] == "operation" for s in snapshots) or rows:
            errors.append("missing operation census or unterminated rows")
        for s in snapshots:
            if s["broker_live"] != sum(s[k] for k in ("ipc_tokens", "shm", "pending", "delivered", "legacy")):
                errors.append("overlapping authoritative inventory")
            if len(s["rows"]) != s["pending"] + s["delivered"] or len({r["fingerprint"] for r in s["rows"]}) != len(s["rows"]):
                errors.append("missing/duplicate authoritative grant owner")
            if s["stale_queue_refs"] != 0 or s["deferred_subset"] > s["reply_targets"] or s["tombstones_subset"] > s["reply_targets"]:
                errors.append("unreconciled IPC references")
            if s["history"] > s["history_capacity"] or s["history_capacity"] > 65536:
                errors.append("invalid history capacity")
            if s["snapshot"] != "scheduler-broker-shards" or s["global_reconciliation"] != "INCOMPLETE":
                errors.append("unsupported snapshot scope claim")
            for row in s["rows"]:
                if row["phase"] != s["phase"] or row["state"] not in ("pending", "delivered"):
                    errors.append("row/snapshot mismatch")
                if min(row[k] for k in ("pid", "incarnation", "vm", "issuer", "issuer_inc", "issuer_vm", "request")) <= 0:
                    errors.append("incomplete grant identity")
                if row["class"] not in ("auth", "account", "vfs") or not re.fullmatch(r"boot-local-[1-9][0-9]*", row["fingerprint"]):
                    errors.append("invented authority class or fingerprint")
                if not 0 <= row["cpu"] < cpus:
                    errors.append("invalid grant CPU")
            if sum(r["state"] == "pending" for r in s["rows"]) != s["pending"]:
                errors.append("state partition mismatch")
        for row in published + reaped + snapshots:
            if not 0 <= row["cpu"] < cpus:
                errors.append("invalid CPU")
        markers = ["[GRANT-F] pid-boundary max=18446744073709551615 last=accepted exhausted=None rollover=denied scope=isolated-real-allocator",
                   "[GRANT-F] pid-contract cycles=8 same_numeric_pid=0 policy=non-reuse scope=ordinary-runtime-spawn PASS"]
        if any(raw.count(marker) != 1 for marker in markers):
            errors.append("missing or duplicate allocator assertions")
        if snapshots and any(snapshots[-1][k] for k in ("pending", "delivered", "retiring_subset")):
            errors.append("bound authority remains at fixture completion")
        if any(x in raw for x in ("PANIC", "panicked at", "client FAIL", "[FAULT]")):
            errors.append("guest failure")
        if any(e["case"] not in ("exit-consume", "exec-consume") for e in lifecycle):
            errors.append("invented lifecycle case")
        for index, case in enumerate(("exit-consume", "exec-consume")):
            events = [e for e in lifecycle if e["case"] == case]
            expected = {"commit", "reaped", "consume-attempt"} | ({"exec"} if index else set())
            if len(events) != len(expected) or {e["checkpoint"] for e in events} != expected:
                errors.append("missing or duplicate lifecycle checkpoint: " + case)
                continue
            by = {e["checkpoint"]: e for e in events}
            c, r, a = (by[k] for k in ("commit", "reaped", "consume-attempt"))
            if c["pid"] != 0xF4F3 + index or any(e["pid"] != c["pid"] or e["incarnation"] != c["incarnation"] for e in events):
                errors.append("lifecycle recipient identity mismatch")
            if a["old_vm"] != c["vm"] or min(c[k] for k in ("vm", "issuer", "issuer_inc", "issuer_vm", "request")) <= 0:
                errors.append("lifecycle VM/issuer/request mismatch")
            if any(e["incarnation"] <= 0 for e in events) or min(a[k] for k in ("issuer", "issuer_inc", "issuer_vm")) <= 0:
                errors.append("missing lifecycle incarnation/consumer identity")
            if not c["position"] < min(r["position"], a["position"]) or (r["exit"] == 0) != bool(index):
                errors.append("lifecycle ordering/exit mismatch")
            if any(r[k] for k in ("pending", "reply", "deferred")) or a["inventory"] > (0 if index else 2):
                errors.append("lifecycle cleanup mismatch")
            if index:
                e = by["exec"]
                if e["old_vm"] != c["vm"] or e["new_vm"] <= 0 or e["new_vm"] == e["old_vm"] or e["retired"] != 2 or e["replay"] != "denied" or not c["position"] < e["position"] < min(r["position"], a["position"]):
                    errors.append("actual exec/VM transition absent or out of order")
            result_marker = "[GRANT-F] consumer-result case=" + case + " allowed=0"
            if raw.count(result_marker) != 1 or raw.find(result_marker) <= a["position"]:
                errors.append("actual VFS consume result absent/out of order")
            if any(not 0 <= e["cpu"] < cpus for e in events):
                errors.append("invalid lifecycle CPU")
        if raw.count("[GRANT-F] consumer-progress retired_exit=1 retired_exec=1 stale_reply=excluded PASS") != 1:
            errors.append("missing independent VFS progress")
        if published and reaped:
            first_publish = min(raw.find("[GRANT-F] pid-published parent=62704 pid=" + str(r["pid"]) + " ") for r in published)
            last_reap = max(raw.find("[GRANT-F] pid-reaped pid=" + str(r["pid"]) + " ") for r in reaped)
            if raw.find("[GRANT-F] census phase=start ") >= first_publish or raw.find("[GRANT-F] census phase=final ") <= last_reap:
                errors.append("census not ordered around actual process lifetime")
    except (KeyError, TypeError, ValueError, IndexError) as error:
        errors.append("incomplete markers: " + str(error))
    return {"classification": "FORCED_AND_FAILED" if errors and (published or lifecycle) else "NOT_FORCED" if errors else "FORCED_AND_PASSED",
            "errors": errors, "pid_contract": "max-retained-plus-one; ordinary dynamic allocation only",
            "forced_numeric_reuse": "NOT_FORCED", "published": published, "reaped": reaped,
            "authority_inventory": snapshots, "lifecycle": lifecycle,
            "cases": {"allocator_non_reuse": "NOT_FORCED" if errors else "PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT",
                      "exit_before_consume": "NOT_FORCED" if errors else "FORCED_AND_PASSED",
                      "exec_before_old_consume": "NOT_FORCED" if errors else "FORCED_AND_PASSED",
                      "inside_serialized_consumption_race": "NOT_FORCED"},
            "global_reconciliation": "BLOCKED",
            "allocator_wide_leak_freedom": "UNMEASURED"}


def main(*, base=BASE, phase="F1.1B-0F", kernel_feature="production_grant_test",
         service_feature="production-grant-test", native_assessor=None, race_assessor=assess_races, terminal="[GRANT-C] COMPLETE PASS", gates=GATES):
    # Later closure phases reuse this collector, builders and PID census.
    BASE = base
    GATES = gates
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--section", choices=("build", "native", "checks", "consolidate"), required=True)
    parser.add_argument("--image-manifest", type=Path)
    parser.add_argument("--cpus", type=int, nargs="+", choices=(1, 2, 4, 12), default=[1])
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    if args.section == "consolidate":
        manifests = sorted(p for p in BASE.rglob("validation-summary.json") if p.parent != BASE)
        results = [{"manifest": str(p.relative_to(ROOT)), "sha256": digest(p), "result": json.loads(p.read_text())} for p in manifests]
        old = json.loads((ROOT / "target/fastlane-f11b0e3t-evidence/baseline-preservation.json").read_text())["old_artifacts"]
        inputs = json.loads((BASE / "audit-inputs.json").read_text())["reports"]
        old.update(inputs)
        changed = [p for p, sha in old.items() if not (ROOT / p).is_file() or digest(ROOT / p) != sha]
        current = sources()
        for record in results:
            recorded = record["result"].get("source_sha256", {})
            record["matches_final_source"] = recorded == current
        summary = {"phase": phase, "updated_utc": datetime.now(timezone.utc).isoformat(), "gates": GATES,
                   "attempts": results, "historical_preservation": {"inventoried": len(old), "changed_or_missing": changed},
                   "source_sha256": current, "report_sha256": digest(ROOT / "docs/ipc/fastlane-f11b0f-pid-reuse-global-accounting.md"),
                   "remaining_blockers": ["remaining native lifecycle orderings", "global userspace/broker reference reconciliation and steady-cycle attribution", "token-history global exhaustion/error delivery and restart/lifetime acceptance"],
                   "independent_blockers": ["N3M original failure unexplained", "D3 failure and incomplete RecoveryBlocked", "Session Authority PARTIAL"],
                   "repository_safety": {"commit": False, "push": False, "destructive_git": False, "production_transport_enabled": False}}
        (BASE / "validation-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(BASE / "validation-summary.json")
        return
    out = BASE / (args.section + "-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ"))
    out.mkdir(parents=True)
    summary = {"phase": phase, "section": args.section, "source_sha256": sources(),
               "gates": GATES, "checks": [], "native": [], "images": {}}
    def save():
        (out / "validation-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    def check(name, command, extra=None, timeout=1200):
        start = time.monotonic()
        with (out / (name + ".log")).open("w") as log:
            log.write(json.dumps({"command": command, "environment": {**ENV, **(extra or {})}}) + "\n"); log.flush()
            try:
                code = subprocess.run(command, cwd=ROOT, env={**os.environ, **ENV, **(extra or {})}, stdout=log, stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        summary["checks"].append({"name": name, "command": command, "environment": {**ENV, **(extra or {})}, "exit_code": code, "timeout": code == 124, "seconds": time.monotonic() - start, "log": str((out / (name + ".log")).relative_to(ROOT))})
        save(); print(name, code, flush=True)
        return code == 0
    try:
        if args.section == "build":
            summary["disk_before"] = subprocess.check_output(["df", "-h", ".", "/tmp"], text=True)
            for package in ("sunlight-init", "sunlightd", "sunlight-tty-server", "sunlight-uac", "sunlight-sessiond", "sunlight-vfs-server"):
                command = ["cargo", "build", "--locked", "--offline", "--release", "-p", package]
                if package in ("sunlight-uac", "sunlight-vfs-server"): command += ["--features", service_feature if package == "sunlight-uac" else "production-grant-test"]
                if not check(package, command, {"RUSTFLAGS": SERVICE_FLAGS}): raise RuntimeError("service build failed")
            for name in ("sunlight-init", "sunlightd", "sunlight-tty-server", "uac_service", "sunlight-sessiond", "sunlight-vfs-server", "grant-probe"):
                shutil.copy2(BUILD / "x86_64-unknown-none/release" / name, ROOT / "target/x86_64-unknown-none/release" / name)
            if not check("kernel", ["cargo", "build", "--locked", "--offline", "-p", "sunlight-kernel", "--bin", "sunlight-kernel", "--features", kernel_feature]): raise RuntimeError("kernel build failed")
            built = BUILD / "x86_64-unknown-none/debug/sunlight-kernel"
            elf, iso = out / "kernel.elf", out / "fixture.iso"
            if not check("strip", ["objcopy", "--strip-debug", str(built), str(elf)]): raise RuntimeError("strip failed")
            scratch = out / "iso-build"; scratch.mkdir()
            shutil.copy2(ROOT / "limine.conf", scratch / "limine.conf")
            if not check("iso", ["bash", "tools/make_hybrid_iso.sh", str(elf), str(iso), str(ROOT / "target/limine"), str(scratch)]): raise RuntimeError("ISO failed")
            embedded = out / "embedded"; embedded.mkdir()
            payloads = {}; raw = elf.read_bytes()
            for name in ("sunlight-init", "sunlightd", "sunlight-tty-server", "uac_service", "sunlight-sessiond", "sunlight-vfs-server", "grant-probe"):
                source = ROOT / "target/x86_64-unknown-none/release" / name
                offset = raw.find(source.read_bytes())
                if offset < 0: raise RuntimeError("embedded binary mismatch: " + name)
                shutil.copy2(source, embedded / name)
                payloads[name] = {"sha256": digest(source), "offset": offset, "bytes": source.stat().st_size}
            summary["images"]["fixture"] = {"kernel_feature": kernel_feature, "iso": str(iso.relative_to(ROOT)), "iso_sha256": digest(iso), "elf_sha256": digest(elf), "embedded": payloads}
        elif args.section == "checks":
            host = ["cargo", "test", "--locked", "--offline", "--target", "x86_64-unknown-linux-gnu"]
            check("kernel-host", host + ["-p", "sunlight-kernel", "--lib", "--", "--test-threads=1"])
            packages = ["sunlight-ipc", "sunlight-uac", "sunlight-sessiond", "sunlight-fs", "sunlightd", "sunlight-audiod", "sunlight-elf", "sunlight-fastlane-ring", "sunlight-fastlane-doorbell"]
            check("services-host", host + sum((["-p", p] for p in packages), []) + ["--lib"])
            check("scheduler-host", ["python3", "tools/test_scheduler_host.py"])
            check("parsers", ["python3", "-m", "unittest", "discover", "-s", "tools", "-p", "test_fastlane_f11b0*.py"])
            check("strict-clippy", ["cargo", "clippy", "--locked", "--offline", "--target", "x86_64-unknown-linux-gnu", "-p", "sunlight-kernel", "--lib", "--", "-D", "warnings"])
            check("media-host", host + ["-p", "sunlight-media", "--lib"])
            check("workspace-format", ["cargo", "fmt", "--all", "--", "--check"])
            check("diff-check", ["git", "diff", "--check"])
            check("miri", ["cargo", "miri", "--version"])
        else:
            if args.image_manifest is None: parser.error("native requires --image-manifest")
            manifest = json.loads(args.image_manifest.read_text()); image = manifest["images"]["fixture"]
            iso = ROOT / image["iso"]; elf = args.image_manifest.parent / "kernel.elf"
            identity = {"source_unchanged": sources() == manifest["source_sha256"], "iso_matches": digest(iso) == image["iso_sha256"], "elf_matches": digest(elf) == image["elf_sha256"], "embedded_match": all(digest(args.image_manifest.parent / "embedded" / name) == row["sha256"] for name, row in image["embedded"].items())}
            if not all(identity.values()): raise RuntimeError("preflight source/image identity mismatch")
            summary["identity"] = identity; summary["image_manifest"] = str(args.image_manifest); summary["images"] = manifest["images"]
            for cpu in args.cpus:
                serial = out / f"production-{cpu}cpu.serial.log"; serial.touch()
                command = ["qemu-system-x86_64", "-accel", "tcg,thread=multi", "-cpu", "max", "-smp", str(cpu), "-m", "1024", "-cdrom", str(iso), "-serial", "file:" + str(serial), "-display", "none", "-nic", "none", "-no-reboot", "-no-shutdown", "-device", "virtio-rng-pci,disable-modern=on", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]
                start = time.monotonic()
                stop = "process-exit"
                with (out / f"qemu-{cpu}cpu.log").open("w") as log:
                    proc = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
                    try:
                        while proc.poll() is None:
                            text = serial.read_text(errors="replace")
                            if any(m in text for m in (terminal, "PANIC", "[GRANT-C] client FAIL")):
                                stop = "terminal-marker"; break
                            if time.monotonic() - start >= args.timeout:
                                stop = "timeout"; break
                            time.sleep(.25)
                    finally:
                        if proc.poll() is None:
                            proc.terminate()
                            try: proc.wait(timeout=5)
                            except subprocess.TimeoutExpired: proc.kill(); proc.wait()
                    code = proc.wait()
                summary["checks"].append({"name": f"qemu-{cpu}cpu", "command": command, "exit_code": code, "collector_stop": stop,
                                          "timeout": stop == "timeout", "seconds": time.monotonic() - start,
                                          "log": str((out / f"qemu-{cpu}cpu.log").relative_to(ROOT))})
                raw = serial.read_text(errors="replace")
                identity["source_unchanged"] = sources() == manifest["source_sha256"]
                result = assess_f(raw, cpu, identity)
                if stop == "timeout":
                    result["errors"].append("collector timeout")
                    result["classification"] = "FORCED_AND_FAILED" if result["lifecycle"] else "NOT_FORCED"
                extra_result = native_assessor(raw, cpu, identity) if native_assessor else None
                if extra_result and extra_result["errors"]:
                    result["errors"].extend(extra_result["errors"])
                    result["classification"] = "FORCED_AND_FAILED"
                summary["native"].append({"closure": extra_result, "cpus": cpu, "command": command, "collector_stop": stop, "serial": str(serial.relative_to(ROOT)), "serial_sha256": digest(serial), "identity": dict(identity), "pid_and_census": result, "delegation": assess_native(raw, cpu), "races": race_assessor(raw, cpu)})
                save(); print(cpu, summary["native"][-1]["pid_and_census"]["classification"], flush=True)
                if any(x in raw for x in ("PANIC", "client FAIL")): break
    finally:
        summary["source_sha256_after"] = sources()
        summary["source_unchanged"] = summary["source_sha256"] == summary["source_sha256_after"]
        save(); print(out / "validation-summary.json", flush=True)
    if not summary["source_unchanged"] or any(row["exit_code"] for row in summary["checks"] if not row["name"].startswith("qemu-")):
        raise SystemExit(1)
    if any(row["pid_and_census"]["errors"] or row["delegation"]["classification"] != "PASS" or row["races"]["classification"] != "FORCED_AND_PASSED" for row in summary["native"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
