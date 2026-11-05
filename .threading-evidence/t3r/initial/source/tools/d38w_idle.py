#!/usr/bin/env python3
"""Bounded D3.8-W paired idle captures from frozen diagnostic ISOs (TCG).

Capture sequentially, with three discarded post-ready windows and three measured
windows per cold boot. No source edits or guest lifecycle changes are made here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import socket
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "target/d38w-evidence"
READY = "[SunlightOS] sunlightd OK"


def host_context():
    paths = ["/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
             "/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference",
             "/proc/sys/kernel/sched_autogroup_enabled"]
    settings = {}
    for path in paths:
        try:
            settings[path] = Path(path).read_text().strip()
        except OSError:
            settings[path] = None
    try:
        model = next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                     if line.startswith("model name"))
    except (OSError, StopIteration):
        model = None
    return dict(platform=platform.platform(), cpu_model=model, settings=settings,
                affinity=sorted(os.sched_getaffinity(0)))


def fields(line):
    return {k: int(v) if v.isdecimal() else v
            for k, v in re.findall(r"(\w+)=([^\s]+)", line)}


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def parse(raw, marker=READY):
    lines = raw.splitlines()
    ready = next((i for i, line in enumerate(lines) if marker in line), None)
    if ready is None:
        raise ValueError("missing SunlightD-ready marker")
    systems = [fields(line) for line in lines[ready:]
               if line.startswith("[CPU-AUDIT] system ")]
    # First post-ready report can straddle startup; discard it and two more.
    selected = systems[3:6]
    if len(selected) != 3:
        raise ValueError("need six post-ready diagnostic windows")
    tasks, cores, samples, memory, ipc, costs = {}, {}, {}, {}, {}, {}
    for line in lines:
        if line.startswith("[WAKE-AUDIT] task "):
            row = fields(line)
            tasks.setdefault(row["at_ns"], {})[(row["pid"], row["gen"])] = row
        elif line.startswith("[WAKE-AUDIT] sample "):
            row = fields(line)
            samples[row["at_ns"]] = row
        elif line.startswith("[WAKE-AUDIT] ipc "):
            row = fields(line)
            ipc[row["at_ns"]] = row
        elif line.startswith("[WAKE-AUDIT] cost "):
            row = fields(line)
            costs[row["at_ns"]] = row["report_ns"]
        elif line.startswith("[CPU-AUDIT] core="):
            row = fields(line)
            cores.setdefault("pending", []).append(row)
        elif line.startswith("[CPU-AUDIT] system "):
            cores[fields(line)["at_ns"]] = cores.pop("pending", [])
        elif line.startswith("[MEM-AUDIT] at_ns="):
            row = fields(line)
            memory[row["at_ns"]] = row
    windows = []
    for system in selected:
        at = system["at_ns"]
        sample = samples[at]
        dt = sample["dt_ns"]
        if "wakes_delta" in sample and at not in costs:
            raise ValueError("diagnostic report is still incomplete")
        for counter, rate_name in (("wakes_delta", "wake_s"), ("switches_delta", "cs_s")):
            if counter in sample:
                system[rate_name] = sample[counter] * 1e9 / dt
        previous = tasks.get(at - dt, {})
        rows, unmatched = [], []
        for identity, task in tasks[at].items():
            old = previous.get(identity)
            if old is None:
                unmatched.append(dict(pid=identity[0], generation=identity[1], name=task["name"]))
                continue  # Never turn a new generation's lifetime into a delta.
            counters = ("wakes", "ipc", "recv_exp", "call_exp", "runtime_ns")
            if any(task[k] < old[k] for k in counters):
                raise ValueError("counter regression within a task incarnation")
            rows.append(dict(pid=identity[0], generation=identity[1], name=task["name"],
                             state=task["state"], **{
                                 k: task[k] - old[k] for k in counters},
                             wakes_s=(task["wakes"] - old["wakes"]) * 1e9 / dt))
        core = cores[at]
        activity = dict(ipc.get(at, {}))
        old_ipc = ipc.get(at - dt, {})
        for key in ("enqueue", "dequeue", "full", "coalesced", "expired", "cancelled", "late", "peer_closed"):
            if key in activity and key in old_ipc:
                activity[key + "_delta"] = activity[key] - old_ipc[key]
        windows.append(dict(system=system, dt_ns=dt, tasks=rows,
                            runnable=sample["runnable"], memory=memory.get(at),
                            ipc=activity, recv_deadlines_armed=sample.get("recv_armed"),
                            call_deadlines_armed=sample.get("call_armed"),
                            diagnostic_report_ns=costs.get(at),
                            unattributed_tasks=unmatched,
                            departed_tasks=[v for k, v in previous.items() if k not in tasks[at]],
                            timer_irqs_s=sample["timer_delta"] * 1e9 / dt if "timer_delta" in sample else sum(c["timer_s"] for c in core),
                            ipc_state_wakes_s=sample["ipc_delta"] * 1e9 / dt if "ipc_delta" in sample else sum(c["ipc_wake_s"] for c in core),
                            attribution_truncated=sample["tasks"] >= sample["limit"]))
    return dict(windows=windows, login_ready="Login screen ready" in raw,
                dialogd_started="[DIALOGD]" in raw or "spawned sunlight-dialogd" in raw,
                clipd_started="[CLIPD] registered" in raw,
                resident_mappings=[fields(l) for l in lines
                                   if l.startswith("[MEM-AUDIT] process ")])


def capture(args):
    out = EVIDENCE / args.phase
    out.mkdir(parents=True, exist_ok=True)
    iso = args.iso.resolve()
    iso_hash = digest(iso)
    for cpu in args.cpus:
        for rep in range(1, args.repetitions + 1):
            stem = out / f"{cpu}cpu-run{rep}"
            serial = stem.with_suffix(".serial.log")
            serial.write_text("")  # Old PASS/data cannot win before QEMU truncates.
            stem.with_suffix(".json").unlink(missing_ok=True)
            command = ["qemu-system-x86_64", "-cdrom", str(iso),
                       "-accel", "tcg,thread=multi", "-cpu", "max",
                       "-m", "1024M", "-smp", str(cpu), "-nic", "none",
                       "-device", "virtio-rng-pci,disable-modern=on",
                       "-device", "qemu-xhci,id=xhci", "-device", "usb-mouse,bus=xhci.0",
                       "-serial", f"file:{serial}", "-display", "none",
                       "-no-reboot", "-no-shutdown"]
            context = dict(command=command, iso_sha256=iso_hash, host=platform.platform(),
                           host_settings=host_context(),
                           affinity=sorted(os.sched_getaffinity(0)), load_before=os.getloadavg(),
                           accelerator="TCG multi-thread", guest_cpus=cpu,
                           qemu=subprocess.check_output(["qemu-system-x86_64", "--version"], text=True),
                           started_utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
            started = time.monotonic()
            result = None
            with stem.with_suffix(".qemu.log").open("w") as log:
                proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                try:
                    while time.monotonic() - started < args.timeout:
                        if proc.poll() is not None:
                            raise RuntimeError(f"QEMU exited {proc.returncode}")
                        raw = serial.read_text(errors="replace") if serial.exists() else ""
                        try:
                            candidate = parse(raw)
                            required = [line for line in (ROOT / "tools/tests/sunlightd.expected").read_text().splitlines()
                                        if line and not line.startswith("#")]
                            if any(marker not in raw for marker in required):
                                raise ValueError("resident boot gate incomplete")
                            if f"({cpu} total logical CPUs)" not in raw:
                                raise ValueError("SMP bring-up marker incomplete")
                            candidate["resident_gate_pass"] = True
                            result = candidate
                            break
                        except (ValueError, KeyError):
                            time.sleep(0.25)
                    if result is None:
                        raise RuntimeError(f"capture timeout: {serial}")
                finally:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait()
                    context.update(wall_seconds=time.monotonic() - started,
                                   load_after=os.getloadavg(), serial_sha256=digest(serial))
                    stem.with_suffix(".context.json").write_text(json.dumps(context, indent=2) + "\n")
            stem.with_suffix(".json").write_text(json.dumps(result, indent=2) + "\n")
            print(f"{args.phase} {cpu} CPU run {rep}: captured three steady windows", flush=True)


def gate(args):
    """Run an isolated functional image; never include it in idle comparisons."""
    out = EVIDENCE / args.gate_label
    out.mkdir(parents=True, exist_ok=True)
    results = []
    for cpu in args.cpus:
        serial = out / f"{cpu}cpu.serial.log"
        serial.write_text("")
        command = ["qemu-system-x86_64", "-cdrom", str(args.iso.resolve()),
                   "-accel", "tcg,thread=multi", "-cpu", "max", "-m", "1024M",
                   "-smp", str(cpu), "-nic", "none", "-device", "virtio-rng-pci,disable-modern=on",
                   "-device", "qemu-xhci,id=xhci", "-device", "usb-mouse,bus=xhci.0",
                   "-serial", f"file:{serial}", "-display", "none", "-no-reboot", "-no-shutdown"]
        passed = False
        with (out / f"{cpu}cpu.qemu.log").open("w") as log:
            proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
            try:
                until = time.monotonic() + args.timeout
                while time.monotonic() < until and proc.poll() is None:
                    raw = serial.read_text(errors="replace")
                    if "[D38W] FAIL" in raw or "KERNEL PANIC" in raw:
                        break
                    if args.marker in raw:
                        required = args.expected.read_text().splitlines() if args.expected else []
                        passed = all(m in raw for m in required if m and not m.startswith("#"))
                        break
                    time.sleep(0.25)
            finally:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
        results.append(dict(cpus=cpu, passed=passed, command=command, host_settings=host_context(),
                            iso_sha256=digest(args.iso), serial_sha256=digest(serial)))
        (out / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        print(f"{args.gate_label} {cpu} CPU: {'PASS' if passed else 'FAIL'}", flush=True)
        if not passed:
            raise RuntimeError(f"functional gate failed: {serial}")


def qmp(stream, name, arguments=None):
    request = {"execute": name}
    if arguments is not None:
        request["arguments"] = arguments
    stream.write(json.dumps(request).encode() + b"\n")
    stream.flush()
    while True:
        line = stream.readline()
        if not line:
            raise RuntimeError("QMP disconnected")
        reply = json.loads(line)
        if "error" in reply:
            raise RuntimeError(reply["error"])
        if "return" in reply:
            return reply["return"]


def key(stream, *names):
    qmp(stream, "send-key", {"keys": [{"type": "qcode", "data": n} for n in names], "hold-time": 60})
    time.sleep(0.15)


def graph(args):
    """Actual graphical sessions, measured independently of resident boot."""
    out = EVIDENCE / f"{args.phase}-graph"
    out.mkdir(parents=True, exist_ok=True)
    for cpu in args.cpus:
        serial = out / f"{cpu}cpu.serial.log"
        serial.write_text("")
        (out / f"{cpu}cpu.json").unlink(missing_ok=True)
        sockpath = Path(f"/tmp/d38w-graph-{os.getpid()}-{cpu}.sock")
        command = ["qemu-system-x86_64", "-cdrom", str(args.iso.resolve()),
                   "-accel", "tcg,thread=multi", "-cpu", "max", "-m", "4096M",
                   "-smp", str(cpu), "-nic", "none", "-device", "virtio-rng-pci,disable-modern=on",
                   "-device", "qemu-xhci,id=xhci", "-device", "usb-mouse,bus=xhci.0",
                   "-serial", f"file:{serial}", "-display", "none", "-no-reboot", "-no-shutdown",
                   "-qmp", f"unix:{sockpath},server=on,wait=off"]
        result = dict(command=command, profiles={}, iso_sha256=digest(args.iso),
                      qemu_version=subprocess.check_output(
                          ["qemu-system-x86_64", "--version"], text=True).strip(),
                      host_settings=host_context(),
                      host=platform.platform(), affinity=sorted(os.sched_getaffinity(0)),
                      load_before=os.getloadavg(), accelerator="TCG multi-thread", guest_cpus=cpu)
        with (out / f"{cpu}cpu.qemu.log").open("w") as log:
            proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
            def wait(predicate, label, timeout=args.timeout):
                until = time.monotonic() + timeout
                while time.monotonic() < until and proc.poll() is None:
                    raw = serial.read_text(errors="replace")
                    if any(m in raw for m in ("KERNEL PANIC", "[FAULT]", "[EXCEPTION]", "panicked at")):
                        raise RuntimeError(f"guest fault during {label}")
                    if predicate(raw):
                        return raw
                    time.sleep(0.25)
                raise RuntimeError(f"timeout: {label}")
            try:
                wait(lambda raw: "Login screen ready" in raw, "login ready")
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                    connection.settimeout(10)
                    connection.connect(str(sockpath))
                    stream = connection.makefile("rwb")
                    stream.readline()
                    qmp(stream, "qmp_capabilities")
                    for k in ("ret", "r", "o", "o", "t", "tab", "spc", "ret"):
                        key(stream, k)
                    wait(lambda raw: "[VORTEX] session component ready" in raw, "desktop ready")
                    wait(lambda raw: "Welcome to SunlightOS phase=window_visible_on_screen" in raw,
                         "initial Welcome window")
                    key(stream, "esc")
                    wait(lambda raw: "name='welcome' reason=exit" in raw, "Welcome closed")
                    def measure(profile, marker):
                        def complete(raw):
                            try:
                                parsed = parse(raw, marker)
                            except (ValueError, KeyError):
                                return False
                            result["profiles"][profile] = parsed
                            return True
                        wait(complete, profile + " idle windows")
                        qmp(stream, "screendump", {"filename": str(out / f"{cpu}cpu-{profile}.ppm")})
                        print(f"{args.phase} {cpu} CPU {profile}: three steady windows", flush=True)
                    measure("desktop", "name='welcome' reason=exit")
                    def launch(name):
                        previous = serial.read_text(errors="replace").count("Run phase=window_visible_on_screen")
                        key(stream, "ctrl", "spc")
                        wait(lambda raw: raw.count("Run phase=window_visible_on_screen") > previous, "Run dialog visible")
                        for c in name:
                            key(stream, {"-": "minus", " ": "spc"}.get(c, c))
                        key(stream, "ret")
                        qmp(stream, "screendump", {"filename": str(out / f"{cpu}cpu-launch.ppm")})
                        wait(lambda raw: f"{name} phase=launch_request_received" in raw,
                             "Run submitted " + name, timeout=15)
                    launch("sunlight-terminal")
                    wait(lambda raw: "phase=first_tab_frame" in raw, "terminal first frame")
                    measure("terminal", "phase=first_tab_frame")
                    # The default image does not install the dialog CLI. Use an
                    # existing native GUI client for real dialog/clipboard IPC.
                    launch("sunlight-edit")
                    wait(lambda raw: "Sunlight Edit phase=window_visible_on_screen" in raw, "editor ready")
                    key(stream, "ctrl", "v")  # Read the empty clipboard, no state mutation.
                    dialog_pid = int(re.findall(r"spawned sunlight-dialogd pid=(\d+)",
                                                serial.read_text(errors="replace"))[-1])
                    def dialog_cycle():
                        before = serial.read_text(errors="replace")
                        previous = before.count("Sunlight Dialog phase=window_visible_on_screen")
                        detached = before.count(f"app_window_detached pid={dialog_pid} ")
                        key(stream, "ctrl", "o")
                        wait(lambda raw: raw.count("Sunlight Dialog phase=window_visible_on_screen") > previous,
                             "next file dialog")
                        qmp(stream, "screendump", {"filename": str(out / f"{cpu}cpu-dialog.ppm")})
                        key(stream, "esc")
                        wait(lambda raw: raw.count(f"app_window_detached pid={dialog_pid} ") > detached,
                             "file dialog cancellation")
                    for _ in range(2):
                        dialog_cycle()
                    # Exit a real client while its third dialog is pending.
                    before = serial.read_text(errors="replace")
                    editor_pid = int(re.findall(r"spawn: /bin/sunlight-edit pid=(\d+)", before)[-1])
                    previous = before.count("Sunlight Dialog phase=window_visible_on_screen")
                    detached = before.count(f"app_window_detached pid={dialog_pid} ")
                    key(stream, "ctrl", "o")
                    wait(lambda raw: raw.count("Sunlight Dialog phase=window_visible_on_screen") > previous,
                         "pending client dialog")
                    # Dialogs are OnTop. Switch to the already logged-in TTY,
                    # terminate exactly this client with the existing shell
                    # command, then return to the graphical session.
                    key(stream, "ctrl", "alt", "f1")
                    wait(lambda raw: "[SESSION] switched to F1 TTY/Login" in raw,
                         "TTY switch for pending client")
                    for c in f"kill {editor_pid}":
                        key(stream, {" ": "spc"}.get(c, c))
                    key(stream, "ret")
                    wait(lambda raw: f"process_mark_finished pid={editor_pid} " in raw,
                         "pending client exit", timeout=30)
                    key(stream, "ctrl", "f2")
                    wait(lambda raw: "[SESSION] switched to F2 GraphicalDesktop" in raw,
                         "return to desktop")
                    qmp(stream, "screendump", {"filename": str(out / f"{cpu}cpu-client-exit.ppm")})
                    key(stream, "esc")
                    wait(lambda raw: raw.count(f"app_window_detached pid={dialog_pid} ") > detached,
                         "orphaned dialog dismissed")
                    # A fresh client must still get a dialog from the same daemon.
                    previous = serial.read_text(errors="replace").count("Sunlight Edit phase=window_visible_on_screen")
                    launch("sunlight-edit")
                    wait(lambda raw: raw.count("Sunlight Edit phase=window_visible_on_screen") > previous,
                         "fresh editor ready")
                    dialog_cycle()
                    key(stream, "esc")
                    current_pid = int(re.findall(r"spawn: /bin/sunlight-edit pid=(\d+)",
                                                 serial.read_text(errors="replace"))[-1])
                    wait(lambda raw: f"process_mark_finished pid={current_pid} " in raw, "editor closure")
                    launch("sunlight-clipman")
                    wait(lambda raw: "Sunlight Clipman phase=window_visible_on_screen" in raw, "clipboard UI")
                    key(stream, "esc")
                    wait(lambda raw: "name='sunlight-clipman' reason=exit" in raw, "clipboard closure")
                    result["dialog_regression"] = dict(consecutive_cancelled=2, pending_client_exit=True,
                                                       subsequent_request=True, daemon_pid=dialog_pid)
                    measure("post-activity", "name='sunlight-clipman' reason=exit")
                    result["complete"] = True
            finally:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
                sockpath.unlink(missing_ok=True)
                result.update(load_after=os.getloadavg(), serial_sha256=digest(serial))
                (out / f"{cpu}cpu.json").write_text(json.dumps(result, indent=2) + "\n")


def service_metrics(windows, name):
    present = [w for w in windows if any(t["name"] == name for t in w["tasks"])]
    incomplete = len(present) != len(windows) or any(
        t["name"] == name for w in windows
        for t in w.get("unattributed_tasks", []) + w.get("departed_tasks", []))
    return dict(
        wakes_s=None if incomplete else statistics.mean(
            sum(t["wakes_s"] for t in w["tasks"] if t["name"] == name) for w in windows),
        windows_attributed=len(present),
        recv_expirations=sum(t["recv_exp"] for w in windows for t in w["tasks"] if t["name"] == name),
        call_expirations=sum(t["call_exp"] for w in windows for t in w["tasks"] if t["name"] == name))


def window_summary(windows, boots):
    names = sorted({t["name"] for w in windows for t in w["tasks"]})
    return dict(
        boots=boots, windows=len(windows), duration_ns=sum(w["dt_ns"] for w in windows),
        wakeups_s=statistics.mean(w["system"]["wake_s"] for w in windows),
        context_switches_s=statistics.mean(w["system"]["cs_s"] for w in windows),
        busy_bp=statistics.mean(w["system"]["busy_bp"] for w in windows),
        diagnostic_report_ns=[w.get("diagnostic_report_ns") for w in windows],
        timer_irqs_s=statistics.mean(w["timer_irqs_s"] for w in windows),
        runnable=[w["runnable"] for w in windows],
        services={name: service_metrics(windows, name) for name in names},
        latency_p50_ms=None, latency_p95_ms=None, latency_p99_ms=None,
        unique_resident_memory_bytes=None)


def summarize():
    phases = {}
    for phase in ("baseline", "after"):
        phases[phase] = {}
        for cpu in (1, 2, 4):
            runs = [json.loads(p.read_text()) for p in sorted((EVIDENCE / phase).glob(f"{cpu}cpu-run*.json"))
                    if not p.name.endswith(".context.json")]
            windows = [w for r in runs for w in r["windows"]]
            if not windows:
                continue
            phases[phase][str(cpu)] = window_summary(windows, len(runs))
    graphical = {}
    for phase in ("baseline", "after"):
        graphical[phase] = {}
        for p in sorted((EVIDENCE / f"{phase}-graph").glob("*cpu.json")):
            run = json.loads(p.read_text())
            if not run.get("complete", False):
                continue
            graphical[phase][p.stem] = {profile: window_summary(value["windows"], 1)
                                       for profile, value in run["profiles"].items()}
    result = dict(phases=phases, graphical=graphical,
                  semantics="Arithmetic mean of per-window rates; see docs/services/sunlightd-d38w-idle-wakeups.md; null means unavailable")
    (EVIDENCE / "idle-summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["capture", "summarize", "gate", "graph"])
    parser.add_argument("--iso", type=Path)
    parser.add_argument("--phase", choices=["baseline", "after"])
    parser.add_argument("--cpus", type=int, nargs="+", default=[1, 2, 4])
    parser.add_argument("--repetitions", type=int, default=2)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--marker", default="[D38W] FINAL PASS")
    parser.add_argument("--expected", type=Path)
    parser.add_argument("--gate-label", default="probe-gate")
    args = parser.parse_args()
    if args.action == "capture":
        if not args.iso or not args.phase:
            parser.error("capture requires --iso and --phase")
        capture(args)
    elif args.action in ("gate", "graph"):
        if not args.iso:
            parser.error("gate requires --iso")
        if args.action == "graph":
            if not args.phase:
                parser.error("graph requires --phase")
            graph(args)
        else:
            gate(args)
    else:
        summarize()


if __name__ == "__main__":
    main()
