#!/usr/bin/env python3
"""Fresh M1/M2 regression matrix for M3; sequential builds preserve service provenance."""
from datetime import datetime, timezone
import json
import mmap
import os
from pathlib import Path
import subprocess

from fastlane_f11b0e_validate import ROOT, digest, sources


def manifest_passed(data, section):
    """Support both actual runner schemas, retaining authenticated ACK gates."""
    if any(c.get("exit_code") != 0 for c in data.get("checks", [])):
        return False
    if data.get("source_hashes_unchanged") is False:
        return False
    if data.get("source_sha256_after", data.get("source_sha256")) != data.get("source_sha256"):
        return False
    if "status" in data:
        return data["status"] == "PASS"
    # The older Account runners have no scalar status. Their successful process
    # exit is already required by run(); additionally require their real image,
    # completed checks, forced native cases and original host acknowledgements.
    if not data.get("images"):
        return False
    if section == "build":
        return bool(data.get("checks"))
    native = data.get("native", [])
    return bool(native) and all(
        r.get("validator", {}).get("classification") == "FORCED_AND_PASSED"
        and r["validator"].get("terminal_protocol") == "PASS"
        and r["validator"].get("host_terminal_acknowledged") is True
        and r["validator"].get("host_ack")
        for r in native
    )


def main():
    out = ROOT / "target/fastlane-m3-evidence" / ("regressions-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ"))
    out.mkdir(parents=True)
    summary = {"status": "RUNNING", "source_sha256": sources(), "commands": [], "manifests": {}, "failures": []}

    def save():
        (out / "orchestration.json").write_text(json.dumps(summary, indent=2) + "\n")

    def run(name, command):
        log = out / (name + ".driver.log")
        environment = {"CARGO_INCREMENTAL": "0", "CARGO_PROFILE_DEV_DEBUG": "0"}
        with log.open("w") as stream:
            stream.write(json.dumps({"command": command, "environment": environment}) + "\n")
            stream.flush()
            code = subprocess.run(command, cwd=ROOT, env={**os.environ, **environment},
                                  stdout=stream, stderr=subprocess.STDOUT).returncode
        summary["commands"].append({"name": name, "command": command, "exit_code": code,
                                    "environment": environment,
                                    "log": str(log), "log_sha256": digest(log)})
        save()
        print(name, code, flush=True)
        if code:
            raise RuntimeError(name + " failed; retained original attempt")

    def manifest(suite, section):
        matches = list((out / suite).glob(section + "-*/validation-summary.json"))
        if len(matches) != 1:
            raise RuntimeError("missing/ambiguous manifest: " + suite + "/" + section)
        data = json.loads(matches[0].read_text())
        if not manifest_passed(data, section) or data["source_sha256"] != summary["source_sha256"]:
            raise RuntimeError("failed or changed-source manifest")
        summary["manifests"][suite + "/" + section] = str(matches[0])
        return str(matches[0])

    def paired(suite, tool, flags):
        command = ["python3", "tools/" + tool, *flags, "--evidence-root", str(out / suite)]
        run(suite + "-build", [*command, "--section", "build"])
        image = manifest(suite, "build")
        run(suite + "-native", [*command, "--section", "native", "--cpus", "2", "4", "12", "--image-manifest", image])
        manifest(suite, "native")

    def attempt(name, action):
        # Independent suites still run after a failure. The aggregate remains
        # FAIL; a passing replay never cancels another suite's failed receipt.
        try:
            action()
        except Exception as error:
            summary["failures"].append({"suite": name, "type": type(error).__name__, "message": str(error)})
            save()
            print(name, "FAIL (continuing independent suites)", flush=True)

    print(out, flush=True)
    save()
    try:
        run("mapping-build", ["python3", "tools/fastlane_mapping_validate.py", "--section", "build", "--evidence-root", str(out / "mapping")])
        image = manifest("mapping", "build")
        # A successful 2-CPU guest precedes expansion. These run M1 + M2 + the
        # M3 aligned-word probes, never a native ring or Doorbell transfer.
        for cpus in ("2", "4", "12"):
            name = "compatibility-" + cpus
            def compatibility():
                run(name, ["python3", "tools/fastlane_m3_validate.py", "--section", "native", "--cpus", cpus,
                           "--image-manifest", image, "--evidence-root", str(out / name)])
                manifest(name, "native")
            attempt(name, compatibility)
        attempt("ipc", lambda: paired("ipc", "fastlane_native_boundary_validate.py", ["--reply-race"]))
        attempt("authority", lambda: paired("authority", "fastlane_f11b0h4_validate.py", ["--cases", "S1", "S2"]))
        base = ["--h4", "--cases", "N6", "--account-exec-cycles", "3"]
        attempt("exec", lambda: paired("exec", "fastlane_f11b0e3_validate.py", [*base, "--account-exec"]))
        for mode in (11, 12, 13):
            attempt("exit-" + str(mode), lambda: paired("exit-" + str(mode), "fastlane_f11b0e3_validate.py", [*base, "--account-cancel", str(mode)]))
        # Direct kernel-only regression builders embed canonical service files.
        # Rebuild normal services first so Account fixture features cannot leak
        # into their images. The production archive is immutable afterward.
        run("production", ["python3", "tools/fastlane_mapping_validate.py", "--section", "production", "--evidence-root", str(out / "production")])
        production = json.loads(Path(manifest("production", "production")).read_text())
        with open(production["image"]["elf"], "rb") as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as data:
            if data.find(b"[FL-M3-WORD]") >= 0 or production["image"]["features"]:
                raise RuntimeError("M3 native fixture leaked into production")
        summary["production_m3_marker_absent"] = True
        for suite, tool, flags in (("scheduler", "test_scheduler_smp.py", []),
                                   ("r2d", "test_remote_wake_smp.py", ["--pin-r2d"]),
                                   ("doorbell", "test_native_doorbell.py", [])):
            attempt(suite, lambda: run(suite, ["python3", "tools/" + tool, *flags, "--cpus", "2", "4", "12", "--evidence-dir", str(out / suite)]))
        def host_checks():
            run("mapping-checks", ["python3", "tools/fastlane_mapping_validate.py", "--section", "checks", "--evidence-root", str(out / "checks")])
            manifest("checks", "checks")
        attempt("mapping-checks", host_checks)
        run("release", ["cargo", "check", "--locked", "--offline", "--release", "-p", "sunlight-kernel", "--bin", "sunlight-kernel"])
        summary["status"] = "FAIL" if summary["failures"] else "PASS"
    except KeyboardInterrupt:
        summary["status"] = "INTERRUPTED"
        summary["failure"] = {"type": "KeyboardInterrupt", "message": "original attempt interrupted; no acceptance"}
    except Exception as error:
        summary["status"] = "FAIL"
        summary["failure"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        summary["source_sha256_after"] = sources()
        if summary["source_sha256"] != summary["source_sha256_after"]:
            summary["status"] = "FAIL"
            summary["source_changed"] = True
        save()
        (out / "artifact-index.json").write_text(json.dumps({str(p.relative_to(out)): digest(p)
            for p in out.rglob("*") if p.is_file() and p.name != "artifact-index.json"}, indent=2) + "\n")
    print(summary["status"], out, flush=True)
    return 0 if summary["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
