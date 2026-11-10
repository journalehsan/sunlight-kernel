#!/usr/bin/env python3
"""Narrow native C driver. Run tools/build_native_c.sh before executable links."""
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SYSROOT = ROOT / "target/native-c/sysroot"
args = sys.argv[1:]
resource = subprocess.check_output(["clang", "-print-resource-dir"], text=True).strip()
# Match tools/targets/x86_64-sunlightos.json: static SysV x86-64/SSE2,
# no red zone, no host headers, libraries, startup, or Linux personality stamp.
command = ["clang", "--target=x86_64-unknown-none-elf", "-march=x86-64",
           "-mno-red-zone", "-mno-mmx", "-mno-avx", "-mno-avx2",
           "-ffreestanding", "-fno-pic", "-fno-pie", "-fno-stack-protector",
           "-nostdinc", "-isystem", resource + "/include",
           "-I" + str(ROOT / "sunlight-libc/include"),
           "--sysroot=" + str(SYSROOT)]
link = bool(args) and not any(x in args for x in ("-E", "-S", "-c", "-fsyntax-only", "--version"))
if link and not (SYSROOT / "lib/libsunlight_native_c.a").is_file():
    sys.exit("native C archive missing; run bash tools/build_native_c.sh")
command += args
if link:
    command += ["-nostdlib", "-static", "-fuse-ld=lld",
                "-Wl,-T," + str(ROOT / "tools/native-c/user-space.ld"),
                "-Wl,--gc-sections,--undefined=_start,--no-undefined,--fatal-warnings",
                str(SYSROOT / "lib/libsunlight_native_c.a")]
os.execvp(command[0], command)
