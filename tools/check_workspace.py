#!/usr/bin/env python3
"""Check every workspace library and binary using its supported platform.

Cargo selects one target/feature graph for --workspace, while this repository
contains host tools, freestanding services, and native std applications. Keep
binary checks separate so host, SVG/std, and allocator features cannot leak
into unrelated freestanding executables.
"""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
HOST = next(line.split(": ", 1)[1] for line in subprocess.check_output(
    ["rustc", "-Vv"], text=True).splitlines() if line.startswith("host: "))
NATIVE = "x86_64-unknown-none"
HOST_PACKAGES = {
    "sunlight-js-cli", "helios-note", "sun-imgc", "sun-font",
    "sunlight-fetch", "sunlight-kv", "sunlight-kvctl", "sunlight-tls",
    "certificatectl", "wiseowl-memory", "wiseowl-memorydb",
    "wiseowl-index", "wiseowl-brain", "sunlight-docserv", "sunlight-docview",
    "rappid-rabbit",
}
SVG_PACKAGES = {
    "sunlight-vortex-shell", "sunlight-files", "sunlight-dialogd",
    "sunlight-control-panel",
}


def main():
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT))
    commands = [["cargo", "check", "--workspace", "--lib", "--target", HOST]]
    for package in metadata["packages"]:
        if not any("bin" in target["kind"] for target in package["targets"]):
            continue
        name = package["name"]
        if name in SVG_PACKAGES:
            command = ["bash", "tools/build_native_std.sh", "check", "-p", name,
                       "--bins", "--features", "native-svg"]
        else:
            target = HOST if name in HOST_PACKAGES else NATIVE
            command = ["cargo", "check", "-p", name, "--bins", "--target", target]
            required = sorted({feature for item in package["targets"]
                               if "bin" in item["kind"]
                               for feature in item.get("required-features", [])})
            if required:
                command += ["--features", ",".join(required)]
        commands.append(command)
    commands += [
        ["bash", "tools/build_native_std.sh", "check", "-p", "wiseowl-brain",
         "--bins", "--no-default-features", "--features", "sunlightos"],
        ["bash", "tools/build_native_std.sh", "check", "-p", "rappid-rabbit", "--bins"],
    ]
    failures = []
    for command in commands:
        print("[workspace-check] " + " ".join(command), flush=True)
        if subprocess.run(command, cwd=ROOT).returncode:
            failures.append(command)
    if failures:
        for command in failures:
            print("FAILED: " + " ".join(command), file=sys.stderr)
        return 1
    print(f"[workspace-check] Passed {len(commands)} checks across all workspace packages.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
