#!/usr/bin/env python3
"""M3 host representation checks and M1/M2 native compatibility receipts.

No native payload transfer or userspace Doorbell acceptance is possible here.
Each run retains its commands, original logs, source snapshot and artifact index.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import subprocess

from fastlane_f11b0e_validate import ROOT, digest, sources
from fastlane_channel_validate import rows

BASE = ROOT / "target/fastlane-m3-evidence"
FIRST = 0x8877665544332211
SECOND = 0x1020304050607080


def assess_words(raw):
    """Additional mapping-width proof; the complete M1/M2 assessor is also required."""
    errors = []
    words = rows(raw, "[FL-M3-WORD] ")
    maps = [r for r in rows(raw, "[FL-MAP] ") if r.get("event") == "map"]
    if len(words) != 6:
        errors.append("six aligned word witnesses required")
    if any("_duplicate" in r for r in words):
        errors.append("duplicate word trace field")
    try:
        if sorted(int(r["region"]) for r in words) != [0, 0, 1, 1, 2, 2]:
            errors.append("page coverage missing")
        if len({r["generation"] for r in words}) != 2:
            errors.append("independent pairs required")
        for r in words:
            if (int(r["offset"]) != 4088 or int(r["first"]) != FIRST
                    or int(r["second"]) != SECOND or r["private_copy"] != "stable"):
                errors.append("invalid aligned private-copy witness")
            selected = []
            for actor, writable in (("writer", "1"), ("reader", "0")):
                matches = [m for m in maps if all(m.get(k) == r.get(k) for k in ("generation", "region"))
                           and m.get("pid") == r[actor] and m.get("va") == r[actor + "_va"]]
                if len(matches) != 1 or matches[0].get("write") != writable:
                    errors.append("word access lacks exact permitted mapping")
                else:
                    selected.append(matches[0])
                if int(r[actor + "_va"]) % 4096 != 0:
                    errors.append("unaligned mapped page")
            if len(selected) == 2 and (selected[0]["frame"] != selected[1]["frame"]
                                       or selected[0]["pid"] == selected[1]["pid"]):
                errors.append("word access lacks distinct peers on the same backing")
        for generation in {r["generation"] for r in words}:
            if sorted(int(r["region"]) for r in words if r["generation"] == generation) != [0, 1, 2]:
                errors.append("incomplete pair coverage")
    except (KeyError, ValueError, TypeError):
        errors.append("invalid word witness fields")
    return {"status": "FAIL" if errors else "PASS", "errors": errors, "witnesses": words,
            "scope": "aligned machine-word loads/stores and private scalar copies on real M2 mappings only"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--section", required=True, choices=("host", "native"))
    parser.add_argument("--cpus", type=int, choices=(2, 4, 12), nargs="+", default=[2])
    parser.add_argument("--image-manifest", type=Path)
    parser.add_argument("--evidence-root", type=Path, default=BASE)
    args = parser.parse_args()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    out = args.evidence_root.resolve() / (args.section + "-" + stamp)
    out.mkdir(parents=True, exist_ok=False)
    summary = {"milestone": "M3.0", "status": "RUNNING", "section": args.section,
               "source_sha256": sources(), "checks": [], "native": [],
               "production_fastlane": "BLOCKED", "native_ring_transfer": "NOT_IMPLEMENTED",
               "userspace_doorbell": "NOT_IMPLEMENTED"}

    def save():
        (out / "validation-summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    def run(name, command, env=None, optional=False):
        environment = {"CARGO_INCREMENTAL": "0", **(env or {})}
        with (out / (name + ".log")).open("w") as log:
            log.write(json.dumps({"command": command, "environment": environment}) + "\n")
            log.flush()
            result = subprocess.run(command, cwd=ROOT, env={**os.environ, **environment},
                                    stdout=log, stderr=subprocess.STDOUT)
        summary["checks"].append({"name": name, "command": command, "environment": environment,
                                  "exit_code": result.returncode, "optional": optional,
                                  "log": str(out / (name + ".log"))})
        save()
        print(name, result.returncode, flush=True)
        if result.returncode and not optional:
            raise RuntimeError(name + " failed; original attempt retained")
        return result.returncode

    save()
    print(out, flush=True)
    try:
        if args.section == "host":
            host = ["--locked", "--offline", "--target", "x86_64-unknown-linux-gnu"]
            run("environment", ["rustc", "-Vv"])
            run("components", ["rustup", "component", "list", "--installed"])
            run("ring-tests", ["cargo", "test", "-p", "sunlight-fastlane-ring", *host])
            run("clippy", ["cargo", "clippy", "-p", "sunlight-fastlane-ring", "--all-targets", *host, "--", "-D", "warnings"])
            run("baremetal", ["cargo", "check", "--locked", "--offline", "-p", "sunlight-fastlane-ring", "--target", "x86_64-unknown-none"])
            run("format", ["rustfmt", "--edition", "2021", "--check", "fastlane-ring/src/hostile.rs", "fastlane-ring/src/hostile/tests.rs",
                           "services/sunlight-uac/src/bin/channel_fixture/mapping.rs"])
            run("parser-controls", ["python3", "-m", "unittest", "discover", "-s", "tools", "-p", "test_fastlane*.py"])
            run("whitespace", ["git", "diff", "--check"])
            miri = run("miri-availability", ["cargo", "miri", "--version"], optional=True)
            if miri == 0:
                run("miri", ["cargo", "miri", "test", "-p", "sunlight-fastlane-ring", "--lib", *host, "hostile::"],
                    {"CARGO_TARGET_DIR": str(ROOT / "target/fastlane-m3-analysis/miri")})
                summary["miri"] = "PASS"
            else:
                summary["miri"] = "NOT_RUN: cargo-miri unavailable; see original availability log"
            for sanitizer in ("address", "thread"):
                build_std = ["-Zbuild-std=std,panic_unwind,test"] if sanitizer == "thread" else []
                run(sanitizer, ["cargo", "test", *build_std, "-p", "sunlight-fastlane-ring", "--lib", *host, "hostile::"],
                    {"CARGO_TARGET_DIR": str(ROOT / "target/fastlane-m3-analysis/" / (sanitizer + "-final")),
                     "CARGO_PROFILE_DEV_PANIC": "unwind", "CARGO_PROFILE_TEST_PANIC": "unwind",
                     "RUSTFLAGS": "-Cpanic=unwind -Zsanitizer=" + sanitizer})
        else:
            if not args.image_manifest:
                raise RuntimeError("--image-manifest is required")
            run("mapping", ["python3", "tools/fastlane_mapping_validate.py", "--section", "native",
                            "--cpus", *map(str, args.cpus), "--image-manifest", str(args.image_manifest.resolve()),
                            "--evidence-root", str(out / "mapping")])
            manifests = list((out / "mapping").glob("native-*/validation-summary.json"))
            if len(manifests) != 1:
                raise RuntimeError("missing original M1/M2 manifest")
            manifest = json.loads(manifests[0].read_text())
            summary["mapping_manifest"] = str(manifests[0])
            for entry in manifest["native"]:
                assessment = assess_words(Path(entry["serial"]).read_text())
                row = {"cpus": entry["cpus"], "serial": entry["serial"],
                       "serial_sha256": entry["serial_sha256"], "assessment": assessment,
                       "m1_m2": entry["assessment"]["status"]}
                summary["native"].append(row)
                if assessment["status"] != "PASS" or row["m1_m2"] != "PASS":
                    raise RuntimeError("native compatibility failed")
                (out / f'{entry["cpus"]}cpu.host-ack.json').write_text(json.dumps(row, indent=2) + "\n")
            if sorted(r["cpus"] for r in summary["native"]) != sorted(args.cpus):
                raise RuntimeError("incomplete CPU coverage")
        summary["status"] = "PASS"
    except Exception as error:
        summary["status"] = "FAIL"
        summary["failure"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        summary["source_sha256_after"] = sources()
        if summary["source_sha256_after"] != summary["source_sha256"]:
            summary["status"] = "FAIL"
            summary["source_changed"] = True
        save()
        for name in summary["source_sha256"]:
            destination = out / "source" / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
        (out / "artifact-index.json").write_text(json.dumps({str(p.relative_to(out)): digest(p)
            for p in out.rglob("*") if p.is_file() and p.name != "artifact-index.json"}, indent=2) + "\n")
    print(summary["status"], out, flush=True)
    return 0 if summary["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
