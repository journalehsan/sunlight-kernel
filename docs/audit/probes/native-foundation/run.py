#!/usr/bin/env python3
"""Retain a fresh baseline and stage-separated native C results in a new directory.

Never executes an ELF on the host. This build-only staging runner does not
invoke a VM; use tools/native-c/guest.py --suite for actual guest acceptance.
"""
import argparse
import csv
from pathlib import Path
import shlex
import subprocess
import sys

BASE = Path(__file__).resolve().parent
PROBES = BASE.parent
ROOT = PROBES.parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--out", type=Path, required=True, help="new evidence directory")
options = parser.parse_args()
OUT = options.out.resolve()
OUT.mkdir(parents=True, exist_ok=False)
(OUT / ".gitignore").write_text("*\n!*.log\n!*.csv\n!.gitignore\n")


def run(command, name, timeout=60):
    try:
        result = subprocess.run(command, cwd=ROOT, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                timeout=timeout)
        code, output = result.returncode, result.stdout
    except (OSError, subprocess.TimeoutExpired) as error:
        code, output = 125, str(error)
    (OUT / (name + ".log")).write_text(
        "$ " + shlex.join(map(str, command)) + "\n" + output + "\nexit=" + str(code) + "\n")
    return code, output


def save_csv(name, rows):
    with (OUT / name).open("w") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


for name, command in (
    ("revision", ["git", "rev-parse", "HEAD"]),
    ("worktree", ["git", "status", "--short"]),
    ("rustc", ["rustc", "-Vv"]),
    ("clang", ["clang", "--version"]),
    ("lld", ["ld.lld", "--version"]),
    ("target", ["cat", "tools/targets/x86_64-sunlightos.json"]),
):
    run(command, name)

# A bare main can appear to link successfully while lacking a usable entry.
run(["clang", "--target=x86_64-unknown-none-elf", "-nostdlib", "-fuse-ld=lld",
     str(BASE / "entry.c"), "-o", str(OUT / "before-entry")], "before-entry-link")
run(["llvm-readelf", "-h", str(OUT / "before-entry")], "before-entry-elf")

resource = subprocess.check_output(["clang", "-print-resource-dir"], text=True).strip()
originals = sorted(p for p in PROBES.glob("*.c") if p.name != "compiler.c") + sorted(PROBES.glob("*.cpp"))
baseline = []
for src in originals:
    # Reproduce the historical default invocation, including its missing runtime.
    command = ["clang++" if src.suffix == ".cpp" else "clang",
               "--target=x86_64-unknown-none-elf", "-isystem", resource + "/include",
               "-nostdinc", "-I" + str(ROOT / "sunlight-libc/include"), str(src),
               "-o", str(OUT / ("before-" + src.stem)), "-nostdlib", "-fuse-ld=lld"]
    code, output = run(command, "before-" + src.stem)
    baseline.append(dict(probe=src.name, build="PASS" if code == 0 else "FAIL",
                         first_error=next((line for line in output.splitlines() if "error:" in line), "")))
save_csv("before.csv", baseline)

code, _ = run(["bash", "tools/build_native_c.sh"], "build-runtime", timeout=600)
if code:
    sys.exit("native runtime packaging failed; see build-runtime.log")
run([sys.executable, str(ROOT / "tools/native-c/cc.py"), "-###",
     str(BASE / "entry.c"), "-o", str(OUT / "entry")], "native-driver-invocation")

rows = []
cases = [("foundation", PROBES / "compiler.c", None)]
cases += [("foundation", BASE / (name + ".c"), status)
          for name, status in (("entry", 0), ("write", 0), ("exit", 23))]
cases += [("original", src, 0) for src in originals]
for group, src, expected in cases:
    name = "after-" + src.stem
    cpp = src.suffix == ".cpp"
    driver = (["clang++", "--target=x86_64-unknown-none-elf", "-nostdinc",
               "-isystem", resource + "/include", "-I" + str(ROOT / "sunlight-libc/include")]
              if cpp else [sys.executable, str(ROOT / "tools/native-c/cc.py")])
    flags = ["-std=c++17" if cpp else "-std=c11", "-ffreestanding", "-fno-builtin"]
    row = dict(group=group, probe=src.name, preprocess="NOT_RUN", compile="NOT_RUN",
               assemble="NOT_RUN", link="NOT_RUN", elf="NOT_RUN", guest="UNKNOWN",
               expected_exit="N/A" if expected is None else expected)
    # -E, C -> assembly, assembly -> object, then final native executable link.
    stages = [ ("preprocess", ["-E", str(src), "-o", str(OUT / (src.stem + ".i"))]),
               ("compile", ["-S", str(src), "-o", str(OUT / (src.stem + ".s"))]),
               ("assemble", ["-c", str(OUT / (src.stem + ".s")), "-o", str(OUT / (src.stem + ".o"))]) ]
    if expected is not None:
        stages.append(("link", [str(OUT / (src.stem + ".o")), "-o", str(OUT / src.stem)]))
    for stage, args in stages:
        code, _ = run(driver + (flags if stage in ("preprocess", "compile") else []) + args,
                      name + "-" + stage)
        row[stage] = "PASS" if code == 0 else "FAIL"
        if code:
            break
    if row["link"] == "PASS":
        binary = OUT / src.stem
        code, headers = run(["llvm-readelf", "-h", "-l", str(binary)], name + "-elf")
        nm_code, undefined = run(["llvm-nm", "--undefined-only", str(binary)], name + "-undefined")
        symbols_code, symbols = run(["llvm-nm", "--defined-only", str(binary)], name + "-symbols")
        addresses = {line.split()[-1]: int(line.split()[0], 16)
                     for line in symbols.splitlines() if len(line.split()) == 3}
        data = binary.read_bytes()
        # ET_EXEC, EM_X86_64, native OSABI, nonzero entry and no interpreter.
        valid = (data[:4] == b"\x7fELF" and data[7] == 0 and data[9:15] != b"HLNX01"
                 and int.from_bytes(data[16:18], "little") == 2
                 and int.from_bytes(data[18:20], "little") == 62
                 and int.from_bytes(data[24:32], "little") == addresses.get("_start", 0) != 0
                 and "INTERP" not in headers and not undefined.strip())
        row["elf"] = "PASS" if code == 0 and nm_code == 0 and symbols_code == 0 and valid else "FAIL"
        selected = ",".join(symbol for symbol in ("_start", "sl_std_start", "main", "sl_std_write", "sl_std_exit")
                            if symbol in addresses)
        run(["llvm-objdump", "-d", "--disassemble-symbols=" + selected,
             str(binary)], name + "-disassembly")
    if expected is None:
        row["guest"] = "N/A"
    rows.append(row)
save_csv("after.csv", rows)
(OUT / "guest.log").write_text(
    "guest=UNKNOWN; no execution command issued.\n"
    "This build-only staging runner does not invoke tools/native-c/guest.py.\n"
    "Run tools/native-c/guest.py --suite for isolated native ELF acceptance.\n"
    "No ELF was executed on the Linux host. exit.c requires guest status 23.\n")
for group in ("foundation", "original"):
    selected = [row for row in rows if row["group"] == group]
    passed = sum(row["assemble"] == "PASS" and row["link"] in ("PASS", "NOT_RUN")
                 and row["elf"] in ("PASS", "NOT_RUN") for row in selected)
    print(f"{group}: build PASS={passed} FAIL={len(selected) - passed}; "
          f"guest UNKNOWN={sum(row['guest'] == 'UNKNOWN' for row in selected)}")
# The legacy API suite deliberately still fails; foundation regressions fail this runner.
sys.exit(int(any("FAIL" in row.values() for row in rows if row["group"] == "foundation")))
