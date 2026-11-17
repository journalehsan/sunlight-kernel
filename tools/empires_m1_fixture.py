#!/usr/bin/env python3
"""Build a separate native UI fixture when the checkout's app resolver is broken.

Temporarily instruments init to spawn the real game ELF after 60 native seconds.
Restores the exact init source before compiling the kernel. Does not patch stat,
filesystem policy, gameplay, input, renderer or kernel source. This is explicitly
fixture launch evidence, never proof that the desktop launcher works.

After use, rebuild sunlight-init and the production kernel/ISO to remove the
fixture from generated artifacts. Generated files belong under target/ only.
"""
import hashlib
import json
import os
import signal
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/empires-m1"
INIT = ROOT / "services/init/src/main.rs"
FLAGS = "-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone"


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    original = INIT.read_bytes()
    assert b"EMPIRES_STARTED" not in original, "restore the previous fixture backup before retrying"
    (OUT / "init.before-fixture.rs").write_bytes(original)
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt("fixture build interrupted; restoring init source")
    signal.signal(signal.SIGTERM, interrupted)
    marker = b"        let now = sunlight_ipc::monotonic_millis();\n"
    assert original.count(marker) == 1, "init probe insertion point changed"
    insert = b'''        static EMPIRES_STARTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if now >= 60_000 && !EMPIRES_STARTED.swap(true, core::sync::atomic::Ordering::SeqCst) {
            debug_log("[EMPIRES-FIXTURE] direct native spawn; desktop resolver NOT validated");
            match sunlight_libc::spawn(b"/bin/sunlight-empires", &[b"sunlight-empires", b"--native-suite"], None) {
                Ok(_) => debug_log("[EMPIRES-FIXTURE] spawn accepted"),
                Err(_) => debug_log("[EMPIRES-FIXTURE] spawn FAILED"),
            }
        }
'''
    patched = original.replace(marker, marker + insert)
    with (OUT / "fixture-build.log").open("w") as log:
        INIT.write_bytes(patched)
        try:
            env = dict(os.environ, RUSTFLAGS=FLAGS)
            subprocess.run(["cargo", "build", "-p", "sunlight-init", "--release"], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        finally:
            if INIT.read_bytes() != patched:
                raise RuntimeError("init changed concurrently; preserving it instead of overwriting")
            INIT.write_bytes(original)
        assert INIT.read_bytes() == original
        subprocess.run(["cargo", "build", "-p", "sunlight-kernel"], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(["bash", "tools/make_hybrid_iso.sh", "target/x86_64-unknown-none/debug/sunlight-kernel", "target/empires-m1/native-fixture.iso", str(ROOT / "target/limine"), str(ROOT)], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    manifest = {"init_source_sha256": hashlib.sha256(original).hexdigest(), "init_source_restored": INIT.read_bytes() == original, "launch": "temporary init fixture, direct native spawn at tick time >=60000 ms", "normal_launcher_validated": False}
    for name, file in [("game", ROOT / "target/x86_64-unknown-none/release/sunlight-empires"), ("kernel", ROOT / "target/x86_64-unknown-none/debug/sunlight-kernel"), ("iso", OUT / "native-fixture.iso")]:
        manifest[name + "_sha256"] = hashlib.sha256(file.read_bytes()).hexdigest()
    (OUT / "fixture-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
