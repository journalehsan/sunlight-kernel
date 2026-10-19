#!/usr/bin/env python3
"""Build matched D3.8-W images, restoring the optimized source even on failure.

The baseline service source must be an actual saved pre-change file. Shared
production services are rebuilt without lifecycle/test injection features.
Only SessionD is changed between the paired images; both use identical audit
instrumentation. An additional isolated functional probe image is retained.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SERVICE = ROOT / "services/sunlight-sessiond/src/main.rs"
FLAGS = "-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone"
SOURCES = ["kernel/src/main.rs", "kernel/src/telemetry.rs", "kernel/src/sched/mod.rs",
           "kernel/src/process/mod.rs", "kernel/src/ipc/mod.rs", "kernel/Cargo.toml",
           "kernel/build.rs", "ipc/src/lib.rs", "services/sunlight-sessiond/src/main.rs",
           "services/sunlight-dialogd/src/main.rs", "tools/d38w_build.py", "tools/d38w_idle.py",
           "services/sunlight-startup-fixture/src/d38w_probe.rs", ".cargo/config.toml",
           "rust-toolchain.toml", "limine.conf"]


def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


@contextmanager
def baseline_source(path):
    saved = SERVICE.read_bytes()
    original = path.read_bytes()
    backup = ROOT / "target/d38w-evidence/optimized-sessiond.saved.rs"
    backup.parent.mkdir(parents=True, exist_ok=True)
    backup.write_bytes(saved)
    try:
        SERVICE.write_bytes(original)
        yield
    finally:
        if SERVICE.read_bytes() == original:
            SERVICE.write_bytes(saved)
        elif SERVICE.read_bytes() != saved:
            raise RuntimeError(f"SessionD changed during baseline build; preserved edits, optimized source saved at {backup}")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--baseline-sessiond", type=Path, required=True)
    p.add_argument("--output", type=Path, default=ROOT / "target/d38w-evidence")
    args = p.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    original = args.baseline_sessiond.resolve()
    if original == SERVICE or "ipc_reply_and_recv_timeout(endpoint, reply, 50)" not in original.read_text():
        p.error("baseline must be the saved original 50 ms SessionD source")
    commands = []
    def run(command, log, env=None):
        commands.append(dict(command=command, rustflags=(env or os.environ).get("RUSTFLAGS", "configured kernel flags")))
        subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    service_env = dict(os.environ, RUSTFLAGS=FLAGS)
    # Remove any stale test injection inherited from an interactive shell.
    service_env.pop("SUNLIGHT_INJECT_PHASE", None)
    service_env.pop("D36R_SCENARIO", None)
    kernel_env = dict(os.environ)
    kernel_env.pop("RUSTFLAGS", None)
    kernel_env.pop("SUNLIGHT_INJECT_PHASE", None)
    with (out / "common-build.log").open("w") as log:
        # These binaries intentionally use different allocator features; a
        # multi-package build would unify libc features and conflict with the
        # manager's allocator. Match the repository's separate service builds.
        for package in ("sunlight-init", "sunlightd", "sunlight-tty-server"):
            run(["cargo", "build", "-p", package, "--release", "--offline"], log, service_env)
    def image(phase, feature):
        dest = out / (phase + "-image")
        dest.mkdir(parents=True, exist_ok=True)
        with (dest / "build.log").open("w") as log:
            if phase == "probe":
                run(["cargo", "build", "-p", "sunlight-startup-fixture", "--bin", "d38w-probe",
                     "--features", "idle-wakeup-test", "--release", "--offline"], log, service_env)
            else:
                run(["cargo", "build", "-p", "sunlight-sessiond", "--release", "--offline"], log, service_env)
            run(["cargo", "build", "-p", "sunlight-kernel", "--features", feature, "--offline"], log, kernel_env)
            shutil.copyfile(ROOT / "target/x86_64-unknown-none/debug/sunlight-kernel", dest / "sunlight-kernel.elf")
            run(["bash", "tools/make_hybrid_iso.sh", str(dest / "sunlight-kernel.elf"), str(dest / "sunlightos.iso")], log)
        for source in SOURCES:
            path = dest / "source" / source
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / source, path)
        (dest / "worktree.patch").write_bytes(subprocess.check_output(["git", "diff"], cwd=ROOT))
        embedded = {}
        for target in ("x86_64-unknown-none", "x86_64-sunlightos", "x86_64-unknown-linux-musl"):
            for binary in (ROOT / "target" / target / "release").glob("*"):
                if binary.is_file():
                    with binary.open("rb") as f:
                        is_elf = f.read(4) == b"\x7fELF"
                    if is_elf:
                        embedded[str(binary.relative_to(ROOT))] = sha(binary)
        (dest / "manifest.json").write_text(json.dumps(dict(
            phase=phase, features=[feature], commands=commands,
            revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            rustc=subprocess.check_output(["rustc", "-vV"], text=True),
            embedded_elf_sha256=embedded,
            sha256={str(f.relative_to(dest)): sha(f) for f in dest.rglob("*") if f.is_file() and f.name != "manifest.json"}), indent=2) + "\n")
        print(f"{phase} image frozen: {dest}", flush=True)
    with baseline_source(original):
        image("baseline", "cpu_accounting_diag")
    image("after", "cpu_accounting_diag")
    image("probe", "idle_wakeup_test")


if __name__ == "__main__":
    main()
