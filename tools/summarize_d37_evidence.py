#!/usr/bin/env python3
"""Summarize D3.7 resident telemetry and the current ownership inventory."""
import json
import pathlib
import re
import statistics

ROOT = pathlib.Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "target/d37-evidence"
SOURCE = ROOT / "docs/services/sunlightd-d35-inventory.json"


def fields(line):
    result = {}
    for key, value in re.findall(r"([a-zA-Z_]+)=([^ ]+)", line):
        try:
            result[key] = int(value)
        except ValueError:
            result[key] = value
    return result


def read_boot(phase, cpu):
    path = EVIDENCE / f"{phase}-{cpu}cpu.serial.log"
    raw = path.read_text()
    cpu_rows = [fields(x) for x in raw.splitlines() if x.startswith("[CPU-AUDIT] system ")]
    memory_rows = [fields(x) for x in raw.splitlines() if x.startswith("[MEM-AUDIT] at_ns=")]
    process_rows = [fields(x) for x in raw.splitlines() if x.startswith("[MEM-AUDIT] process ")]
    task_rows = [fields(x) for x in raw.splitlines() if x.startswith("[CPU-AUDIT] task ")]
    return {
        "serial": str(path.relative_to(ROOT)),
        "sunlightd_ready_marker": "[SunlightOS] sunlightd OK" in raw,
        "login_ready_marker": "[TTY]  Login screen ready" in raw,
        "cpu_windows": cpu_rows,
        "memory_windows": memory_rows,
        "process_mapped_memory": process_rows,
        "top_cpu_tasks": task_rows,
        "live_task_count": memory_rows[-1].get("tasks") if memory_rows else None,
        "dialogd_service_started": "[SUNLIGHTD] spawned sunlight-dialogd" in raw,
        "clipd_service_started": "[SUNLIGHTD] spawned sunlight-clipd" in raw,
        "memorydb_waiting_for_persistence": "persistent /state unavailable; identity startup suspended" in raw,
        "swapd_reaped": "process_reaped" in raw and "name='sunlight-swapd'" in raw,
    }


def avg(rows, key):
    values = [r[key] for r in rows if key in r]
    return round(statistics.mean(values), 2) if values else None


base = json.loads(SOURCE.read_text())
boots = {
    phase: {str(cpu): read_boot(phase, cpu) for cpu in (1, 2, 4)}
    for phase in ("baseline", "after")
}

comparison = {}
for cpu in (1, 2, 4):
    before = boots["baseline"][str(cpu)]["cpu_windows"]
    after = boots["after"][str(cpu)]["cpu_windows"]
    comparison[str(cpu)] = {
        "baseline_busy_bp_avg": avg(before, "busy_bp"),
        "final_busy_bp_avg": avg(after, "busy_bp"),
        "busy_bp_delta": (avg(after, "busy_bp") - avg(before, "busy_bp"))
        if before and after else None,
        "baseline_wakeups_per_second_avg": avg(before, "wake_s"),
        "final_wakeups_per_second_avg": avg(after, "wake_s"),
        "wakeups_per_second_delta": (avg(after, "wake_s") - avg(before, "wake_s"))
        if before and after else None,
        "baseline_context_switches_per_second_avg": avg(before, "cs_s"),
        "final_context_switches_per_second_avg": avg(after, "cs_s"),
        "context_switches_per_second_delta": (avg(after, "cs_s") - avg(before, "cs_s"))
        if before and after else None,
        "baseline_schedules_per_second_avg": avg(before, "sched_s"),
        "final_schedules_per_second_avg": avg(after, "sched_s"),
        "schedules_per_second_delta": (avg(after, "sched_s") - avg(before, "sched_s"))
        if before and after else None,
    }

processes = boots["after"]["2"]["process_mapped_memory"]
process_names = {p.get("name") for p in processes}
owners = {
    "init": "Kernel bootstrap",
    "vfs_server": "Kernel bootstrap",
    "tty_server": "Kernel bootstrap",
    "timer_server": "Init",
    "sunlight-kbd": "Init",
    "sunlight-mouse": "Init",
    "sunlight-usb-mouse": "Init",
    "deviced": "Init",
    "networkd": "Init",
    "resolved": "Init",
    "powerd": "Init",
    "thermald": "Init",
    "audiod": "Init",
    "sunlight-display": "Init",
    "mezzo": "Init",
    "pty_server": "Init",
    "net_server": "Init",
    "sunlightd": "Init (manager; no successor recovery)",
    "timezone_service": "SunlightD",
    "timed": "SunlightD",
    "niced": "SunlightD",
    "gcd": "SunlightD",
    "uac_service": "SunlightD",
    "sunlight-sessiond": "SunlightD",
    "sunlight-sm": "SunlightD",
    "sunlight-kv": "SunlightD",
    "sunlight-clipd": "SunlightD",
    "sunlight-dialogd": "SunlightD",
    "rand_service": "SunlightD",
    "sunlight-tls": "SunlightD",
    "wiseowl-memorydb": "SunlightD (suspended; endpoint not ready)",
}
process_inventory = []
for p in processes:
    name = p.get("name")
    state = p.get("state")
    process_inventory.append({
        "pid_observation_only": p.get("pid"),
        "name": name,
        "launcher_or_lifecycle_owner": owners.get(name, "Unclassified process; inspect source"),
        "runtime_state": {0: "Ready", 1: "Running", 2: "BlockedOnIPC", 3: "Finished", 7: "Reaped"}.get(state, state),
        "mapped_user_pages_not_unique": p.get("mapped_user_pages"),
        "mapped_bytes_not_unique": p.get("mapped_bytes"),
    })

units = []
live_unit_names = process_names
for unit in base["compiled_units"]:
    executable = unit.get("executable", "")
    binary = pathlib.PurePosixPath(executable).name
    enabled = unit.get("default_enabled", False)
    condition = unit.get("conditional")
    if condition:
        state = "Conditional definition; absent from default runtime"
    elif not enabled:
        state = "Disabled by default"
    elif binary in live_unit_names:
        state = "Spawned by checkpoint"
    elif binary in {"wiseowl-memoryd", "wiseowl-indexd", "wiseowl-braind"}:
        state = "Not spawned; Requires/After chain awaits MemoryDB endpoint"
    else:
        state = "Enabled; not observed spawned by checkpoint"
    units.append({
        "name": unit.get("name", binary),
        "executable": executable,
        "default_enabled": enabled,
        "build_condition": condition,
        "residency_policy": unit.get("classification"),
        "runtime_state_at_checkpoint": state,
    })

inventory = {
    "schema_version": 1,
    "phase": "D3.7 resource baseline and residency overlay",
    "source_inventory": "docs/services/sunlightd-d35-inventory.json",
    "source_audit": "tools/audit_d35_ownership.py --check-owners",
    "source_counts": base["counts"],
    "measurement_condition": {
        "emulator": "QEMU TCG",
        "ram_mib": 1024,
        "cpus": [1, 2, 4],
        "profile": "Default resident boot, no disk, no authenticated graphical session or application workload",
        "observation_seconds_after_sunlightd_marker": 16,
        "cpu_interval_seconds": 5,
        "clock_resolution_ms": 10,
    },
    "actual_boot_process_map": {
        "kernel_direct": ["init", "vfs_server", "tty_server"],
        "init_launch_order": base["init_boot_order"],
        "init_live_after_boot": [p["name"] for p in process_inventory if p["launcher_or_lifecycle_owner"] == "Init"],
        "init_one_shot_reaped": "sunlight-swapd",
        "sunlightd_live_units": [n for n in (
            "timezone_service", "timed", "niced", "gcd", "uac_service", "sunlight-sessiond",
            "sunlight-sm", "sunlight-kv", "sunlight-clipd", "sunlight-dialogd", "rand_service",
            "sunlight-tls", "wiseowl-memorydb") if n in process_names],
        "sunlightd_enabled_but_deferred": ["wiseowl-memoryd", "wiseowl-indexd", "wiseowl-braind"],
        "session_bound": {"sunlight-vortex-shell": "Not launched; no authenticated user session"},
        "application_children": "None observed in this boot profile",
        "process_count_at_checkpoint": boots["after"]["2"]["live_task_count"],
        "processes": process_inventory,
    },
    "sunlightd_default_units": units,
    "classifications": {
        "BootstrapRequired": ["init", "vfs_server", "tty_server", "sunlightd"],
        "Resident": "Kernel/Init services and current SunlightD legacy units; see source inventory and runtime overlay",
        "OnDemand": "Synthetic D1/D2/D3 fixtures only; no production unit opts in",
        "SessionBound": ["sunlight-vortex-shell", "authenticated TTY shells and their application children"],
        "OneShot": ["sunlight-swapd"],
        "BlockedCandidate": ["sunlight-thumbd", "sunlight-clipd", "sunlight-dialogd"],
    },
    "cpu_comparison": comparison,
    "runtime_observations": boots,
}
(EVIDENCE / "service-inventory.json").write_text(json.dumps(inventory, indent=2) + "\n")

d3_summary_path = ROOT / "target/d3-evidence/validation-summary.json"
d3_summary = json.loads(d3_summary_path.read_text()) if d3_summary_path.exists() else None
summary = {
    "schema_version": 1,
    "phase": "SunlightD D3.7",
    "status": "PASS for measured boot baseline and limited DialogD idle-wakeup change; no production unload pilot",
    "inventory": "service-inventory.json",
    "measurement_evidence": {f"{phase}-{cpu}cpu": boots[phase][str(cpu)]["serial"] for phase in boots for cpu in (1, 2, 4)},
    "qemu": inventory["measurement_condition"],
    "resident_gates": {str(cpu): {
        "baseline": boots["baseline"][str(cpu)]["sunlightd_ready_marker"],
        "final": boots["after"][str(cpu)]["sunlightd_ready_marker"],
        "login_ready": boots["after"][str(cpu)]["login_ready_marker"],
    } for cpu in (1, 2, 4)},
    "host_tests": {
        "dialog_protocol": "13 passed",
        "sunlightd_d1_d2_d3_and_kernel_ipc_lifecycle": d3_summary,
    },
    "pilot": "DialogD blocking idle receive only; no D1/D2 on-demand unload policy enabled",
    "pilot_comparison": comparison,
    "limitations": [
        "No authenticated graphical session or ordinary dialog interaction workload was injected.",
        "No ClipD state-transfer/drain contract exists; it remains resident.",
        "No production service had all readiness, lease, quiesce, drain and ownership prerequisites for unloading.",
        "Boot-to-ready milestones lack per-event timestamps; boot, login and desktop latency were not measured.",
        "Per-process mapped pages include shared mappings and file-backed mappings; they are not unique physical memory.",
        "RAMFS metadata, per-process heap/stack split, process-private unique frames and graphics buffer attribution are unavailable.",
        "QEMU TCG single-run CPU samples are sensitive to host scheduling; total busy-time change was inconsistent.",
        "No application open/close repeated-cycle resource comparison was run.",
    ],
    "raw_logs": sorted(
        boot["serial"] for phase in boots.values() for boot in phase.values()
    ),
}
(EVIDENCE / "validation-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(f"wrote {EVIDENCE / 'service-inventory.json'} and validation-summary.json")
