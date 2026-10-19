"""Boot the D1 experiment on each requested CPU count; retain serial evidence."""
import argparse
import json
import pathlib
import re
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument("--cpus", nargs="+", type=int, default=[1, 2, 4])
parser.add_argument("--timeout", type=int, default=180)
parser.add_argument("--iso", default="target/sunlightd-d1.iso")
parser.add_argument("--evidence", default="target/d1-evidence")
parser.add_argument("--d2", action="store_true")
parser.add_argument("--d3", action="store_true")
args = parser.parse_args()
evidence = pathlib.Path(args.evidence)
evidence.mkdir(parents=True, exist_ok=True)
expected = ["inactive discoverable at boot", "unauthorized bearer rejected without activation",
            "concurrent single flight and cancellation", "ready reuse and direct IPC identity",
            "activation does not grant invocation", "spawn failure crash before ready and timeout", "dependency activation",
            "destroyed endpoint invalidates live instance", "restart fresh identity stale capability rejected", "ready crash invalidates discovery",
            "resident services unaffected", "COMPLETE"]
measurements = []
warm_measurements = []
d2_measurements = []
d3_measurements = []
for cpus in args.cpus:
    if cpus not in (1, 2, 4):
        parser.error("D1 supported matrix is 1, 2, 4 CPUs")
    log = evidence / f"serial-{cpus}cpu.log"
    log.write_text("")
    command = ["qemu-system-x86_64", "-cdrom", args.iso, "-serial", f"file:{log}",
               "-display", "none", "-m", "1024M", "-smp", str(cpus),
               "-device", "virtio-rng-pci,disable-modern=on", "-no-reboot", "-no-shutdown"]
    accelerator = "tcg"
    if pathlib.Path("/dev/kvm").exists():
        command += ["-enable-kvm", "-cpu", "host"]
        accelerator = "kvm"
    with (evidence / f"qemu-{cpus}cpu.log").open("w") as stderr:
        process = subprocess.Popen(command, stdout=stderr, stderr=stderr)
        try:
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline:
                output = log.read_text(errors="replace") if log.exists() else ""
                if re.search(r"\[D[123]-TEST\].*FAIL|KERNEL PANIC|\[SUNLIGHTD\] PANIC", output):
                    raise RuntimeError(f"{cpus} CPU native failure\n{output[-7000:]}")
                if "[D1-TEST] COMPLETE PASS" in output and (not args.d2 or "[D2-TEST] COMPLETE PASS" in output) and (not args.d3 or "[D3-TEST] COMPLETE PASS" in output):
                    missing = [m for m in expected if f"[D1-TEST] {m} PASS" not in output]
                    if missing:
                        raise RuntimeError(f"Missing native assertions: {missing}")
                    if args.d2:
                        required = ["inactive default and administrative activation retained",
                            "authenticated idle quiesce and lease ownership",
                            "busy persistent state minimum uptime and grace cancellation",
                            "concurrent work activation stopping and restart barrier",
                            "concurrent reactivation single flight fresh endpoint",
                            "client death cannot permanently leak residency lease",
                            "stale acknowledgement quiesce timeout and exit timeout retain runtime",
                            "crash while idle is not successful unload",
                            "dependency holds residency until consumer exit",
                            "repeated cycles private reclamation shared references and idle CPU",
                            "resident compatibility"]
                        missing = [s for s in required if f"[D2-TEST] {s} PASS" not in output]
                        if missing:
                            raise RuntimeError(f"Missing D2 assertions: {missing}")
                        samples = [dict(cpus=cpus, accelerator=accelerator,
                            **{key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", line)})
                            for line in output.splitlines() if "[D2-MEM]" in line]
                        if len(samples) != 6:
                            raise RuntimeError(f"Expected six complete D2 PMM samples, got {len(samples)}")
                        d2_measurements.extend(samples)
                    if args.d3:
                        required = ["ordinary scoped activation without discovery lookup invocation or management",
                            "concurrent shared transitive holds graceful release and stable cycles",
                            "dependency crash quarantines parents until confirmed cleanup",
                            "crashed dependent cannot permanently pin providers",
                            "dependency death during startup releases partial graph",
                            "client death during activation leaves no dependency holds",
                            "shared provider survives one consumer shutdown"]
                        missing = [s for s in required if f"[D3-TEST] {s} PASS" not in output]
                        if missing: raise RuntimeError(f"Missing D3 assertions: {missing}")
                        samples = [dict(cpus=cpus, accelerator=accelerator,
                            **{key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", line)})
                            for line in output.splitlines() if "[D3-MEM]" in line]
                        if len(samples) != 6: raise RuntimeError("Expected six graph cycles")
                        d3_measurements.extend(samples)
                    foundation = [line for line in pathlib.Path("tools/tests/phase2_6.expected").read_text().splitlines()
                                  if line and not line.startswith(("#", "[SMP]"))]
                    foundation.append(f"[SMP] Phase-1 bring-up complete: {cpus - 1} AP(s) online ({cpus} total logical CPUs)")
                    missing = [line for line in foundation if line not in output]
                    if missing:
                        raise RuntimeError(f"IPC foundation regression: {missing}")
                    warm_measurements.extend(dict(cpus=cpus, accelerator=accelerator, elapsed_ms=int(ms))
                        for ms in re.findall(r"\[D1-TEST\] warm_activation_ms=(\d+)", output))
                    for match in re.finditer(r"\[D1\] ready unit=(\S+) generation=(\d+) pid=(\d+) process_generation=(\d+) endpoint=(\d+) cold_ms=(\d+) ready_ms=(\d+)", output):
                        unit, generation, pid, proc_gen, ep, cold, ready = match.groups()
                        measurements.append(dict(cpus=cpus, accelerator=accelerator, unit=unit,
                            generation=int(generation), pid=int(pid), process_generation=int(proc_gen),
                            endpoint=int(ep), cold_ms=int(cold), ready_ms=int(ready)))
                    print(f"D1 {cpus} CPU: PASS ({accelerator}); serial: {log}", flush=True)
                    break
                if process.poll() is not None:
                    raise RuntimeError(f"QEMU exited: {process.returncode}; see {log}")
                time.sleep(0.2)
            else:
                raise RuntimeError(f"D1 {cpus} CPU timed out; see {log}\n{output[-5000:]}")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
(evidence / "latency.json").write_text(json.dumps(measurements, indent=2) + "\n")

(evidence / "warm-latency.json").write_text(json.dumps(warm_measurements, indent=2) + "\n")
if args.d2:
    (evidence / "idle-memory.json").write_text(json.dumps(d2_measurements, indent=2) + "\n")
(evidence / "benchmark-context.json").write_text(json.dumps(dict(clock_resolution_ms=10,
    synthetic_ready_delay_ms=250, memory_mib=1024, samples="one guest boot per CPU count"), indent=2) + "\n")

if args.d3:
    (evidence / "dependency-memory.json").write_text(json.dumps(d3_measurements, indent=2) + "\n")
