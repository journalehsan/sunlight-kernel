"""Inspect the pinned Qalculate snapshot without invoking its interpreter."""

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import re
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    snapshot = args.snapshot.resolve()
    bundle = snapshot / "Qalculate.sunapp"
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    commands = []

    def run(*argv):
        commands.append({"cwd": str(bundle), "argv": argv})
        return subprocess.check_output(argv, cwd=bundle, text=True,
                                       env={**os.environ, "DEBUGINFOD_URLS": "", "LC_ALL": "C"})

    checks = run("sha256sum", "--strict", "-c", "../SHA256SUMS")
    (output / "historical-hash-check.txt").write_text(checks)
    historical = (snapshot / "SHA256SUMS").read_text()
    (output / "staged-SHA256SUMS").write_text(historical)
    listing = (snapshot / "staged-loader-list.log").read_text()
    (output / "historical-loader-order.txt").write_text(listing)
    roots = []
    listed = set()
    for line in listing.splitlines():
        if line.startswith("Program/"):
            roots.append(line)
        elif line.strip() and "linux-vdso.so.1" not in line:
            match = re.search(r"(?:^\s+|\s=>\s)(?:\S*/Qalculate\.sunapp/)?(Program/\S+)\s+\(", line)
            if not match:
                raise ValueError(f"Unaccounted loader entry: {line}")
            listed.add(match[1])
    paths = sorted(set(roots) | listed)
    pinned = {line.split(maxsplit=1)[1].removeprefix("./")
              for line in historical.splitlines()}
    if not set(paths) <= pinned:
        raise ValueError("Loader listing contains unpinned artifacts")
    hashes = run("sha256sum", "--", *paths)
    (output / "SHA256SUMS").write_text(hashes)
    digests = {line.split()[1]: line.split()[0] for line in hashes.splitlines()}
    records = {}
    summaries = {
        key: [] for key in ("tls-models", "relocations", "constructors-destructors",
                            "cxx-unwind", "symbol-versions", "syscalls")
    }
    syscall_sites = re.compile(r"\bsyscall\s*$")
    runtime_names = re.compile(
        r"mmap|munmap|mprotect|\bbrk\b|futex|clone|arch_prctl|set_tid_address|"
        r"set_robust_list|sigaction|sigprocmask|sigreturn|clock_gettime|openat|"
        r"\b(?:read|write|close|ioctl|poll|ppoll|stat|statx|statfs)\b|"
        r"epoll|getrandom|prlimit|sched_yield|exit|locale|iconv|getenv|getauxval"
    )
    for number, path in enumerate(paths):
        label = f"{number:02d}-{Path(path).name}"
        elf = run("readelf", "-W", "-h", "-l", "-S", "-d", "-r", "-s", "-V", "-n", "--", path)
        (output / f"{label}.elf.txt").write_text(elf)
        _, _, dynsym = elf.partition("Symbol table '.dynsym'")
        dynsym = dynsym.split("Symbol table '", 1)[0]
        symbols = [line for line in dynsym.splitlines()
                   if re.match(r"\s*\d+:\s+[0-9a-f]+\s+", line)]
        undefined = [line for line in symbols if " UND " in line]
        exported = [line for line in symbols if " UND " not in line
                    and re.search(r"\b(GLOBAL|WEAK|UNIQUE)\s+(DEFAULT|PROTECTED)\b", line)]
        for suffix, rows in (("undefined", undefined), ("exported", exported)):
            (output / f"{label}.{suffix}.txt").write_text("\n".join(rows) + "\n")
        relocations = Counter(re.findall(r"\bR_X86_64_\w+", elf))
        code = run("objdump", "-dw", "--", path)
        context = []
        function = ""
        last = []
        for line in code.splitlines():
            if re.match(r"^[0-9a-f]+ <", line):
                function = line
                last.clear()
            if syscall_sites.search(line) or re.search(r"\bcall.*__tls_get_addr", line):
                context.extend((function, *last, line, ""))
            last = [*last[-5:], line]
        (output / f"{label}.instruction-sites.txt").write_text("\n".join(context))
        needed = re.findall(r"\(NEEDED\).*\[(.*?)\]", elf)
        soname = re.findall(r"\(SONAME\).*\[(.*?)\]", elf)
        interpreter = re.findall(r"Requesting program interpreter: (.*?)\]", elf)
        records[path] = {
            "path": str(bundle / path), "resolved_path": str((bundle / path).resolve()),
            "sha256": digests[path], "label": label, "needed_in_order": needed,
            "soname": soname, "interpreter": interpreter, "relocations": dict(relocations),
            "tls": re.findall(r"^\s+TLS\s+(.+)$", elf, re.M),
            "load": re.findall(r"^\s+LOAD\s+(.+)$", elf, re.M),
            "ifunc_exports": [line for line in exported if " IFUNC " in line],
        }
        summaries["relocations"].append(f"{path}: {dict(relocations)}")
        for key, pattern in {
            "constructors-destructors": r"\((?:PREINIT_ARRAY|INIT|FINI)\w*\)",
            "cxx-unwind": r"\.eh_frame|\.gcc_except_table|_Unwind_|__cxa_|__gxx_personality|_ZTI|_ZTV",
            "symbol-versions": r"\b(?:File:|Name:)\s+\S+",
            "tls-models": r"^\s+TLS\s+0x|R_X86_64_\w*(?:TLS|TPOFF|TPMOD)\w*|__tls_get_addr",
        }.items():
            summaries[key].extend((f"\n{path}", *(line for line in elf.splitlines()
                                                  if re.search(pattern, line))))
        summaries["syscalls"].extend((f"\n{path}", *(line for line in undefined
                                                  if runtime_names.search(line))))
    providers = {Path(path).name: path for path in records if path.startswith("Program/lib/")}
    for path, row in records.items():
        row["edges_in_order"] = [providers[name] for name in row["needed_in_order"]]
        row["interpreter_edges"] = [providers[Path(name).name] for name in row["interpreter"]]
    pending = list(roots)
    reached = set()
    while pending:
        path = pending.pop()
        if path not in reached:
            reached.add(path)
            pending.extend(records[path]["edges_in_order"] + records[path]["interpreter_edges"])
    if reached != set(paths):
        raise ValueError(f"Unused listed artifacts: {set(paths) - reached}")
    for key, rows in summaries.items():
        (output / f"{key}.txt").write_text("\n".join(rows) + "\n")
    (output / "artifact-manifest.json").write_text(json.dumps(records, indent=2) + "\n")
    (output / "dependency-graph.json").write_text(json.dumps({
        "roots": roots, "edges": {path: row["edges_in_order"] for path, row in records.items()},
        "interpreter_edges": {path: row["interpreter_edges"] for path, row in records.items()
                              if row["interpreter_edges"]},
        "host_synthetic_object": "linux-vdso.so.1",
    }, indent=2) + "\n")
    run("sha256sum", "--strict", "-c", str(output / "SHA256SUMS"))
    (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")


if __name__ == "__main__":
    main()
