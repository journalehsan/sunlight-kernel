"""Gate ELF shape before boot and ABI evidence after guest acceptance."""
import json
from pathlib import Path
import re
import sys


def require(condition, detail):
    if not condition:
        sys.exit(f"FAIL: {detail}")


out = Path(sys.argv[2])
fixtures = ("musl", "native")
if sys.argv[1] == "prepare":
    cases = []
    for fixture in fixtures:
        name = f"parity-{fixture}"
        elf = out / f"{fixture}.elf"
        ident = elf.read_bytes()[:16]
        linux = fixture == "musl"
        require(ident[7] == (3 if linux else 0) and
                (ident[9:15] == b"HLNX01" if linux else ident[8:16] == bytes(8)), name + " ELF ABI")
        info = (out / f"{fixture}.readelf").read_text()
        require(re.search(r"Type:\s+EXEC\b", info) and
                re.search(r"(?m)^\s+TLS\s+\S+.*\bR\s+0x", info) and
                re.search(r"\bTLS\s+LOCAL\s+DEFAULT\s+\d+\s+(?:_ZL5)?local$", info, re.M),
                name + " ET_EXEC, PT_TLS and fixture TLS symbol")
        require(not re.search(r"(?m)^\s+(INTERP|DYNAMIC)\s+", info) and "(NEEDED)" not in info,
                name + " static layout")
        if linux:
            link_map = (out / "musl.map").read_text()
            require(re.search(r"\S*musl\S*/libc\.a\(pthread_create\.", link_map),
                    "musl pthread_create archive provenance")
        else:
            for symbol in ("__tls_tdata_start", "__tls_tdata_end", "__tls_tbss_end", "__tls_align"):
                require(re.search(rf"\b{symbol}$", info, re.M), f"native TLS image symbol {symbol}")
            command = (out / "native-link.log").read_text()
            for archive in ("cxx", "eh", "native_c"):
                require(f"libsunlight_{archive}.a" in command, f"native link line: {archive}")
        cases.append(dict(name=name, abi="linux" if linux else "native", elf=str(elf),
                          expected_exit=0, stderr="", stdout="".join(
                              f"{name} {marker}\n" for marker in ("START", "FILE PASS", "TLS PASS", "JOIN PASS"))))
    (out / "cases.json").write_text(json.dumps(cases, indent=2) + "\n")
    print("PASS: both fixture ELF inspections")
elif sys.argv[1] == "trace":
    serial = (out / "guest/serial.log").read_text(errors="replace")
    for index, fixture in enumerate(fixtures):
        linux = fixture == "musl"
        abi = "linux" if linux else "native"
        starts = re.findall(rf"^\[NATIVE-C-START\] case={index} name=parity-{fixture} pid=(\d+) "
                            rf"bytes=\d+ owned_frames=\d+ uid=1000 abi={abi}$", serial, re.M)
        require(len(starts) == 1, fixture + " loader personality")
        pid = starts[0]
        opening = (r"linux=true nr=(?:2 handler=sys_open|257 handler=sys_linux_openat)" if linux
                   else r"linux=false nr=40 handler=sys_open")
        opens = re.findall(rf"^\[PARITY-OPEN\] pid={pid} {opening} backend=kernel-vfs$", serial, re.M)
        require(len(opens) == 2, fixture + " create/reopen routing")
        spawn = (rf"\[HELIOS-THREAD\] clone tgid={pid}" if linux else
                 rf"\[SYSCALL\] thread_spawn: parent={pid}")
        tids = re.findall(rf"^{spawn} tid=(\d+) ", serial, re.M)
        require(len(tids) == 2 and len(set(tids)) == 2 and pid not in tids, fixture + " worker admission")
        print(f"PASS: {fixture} pid={pid} {abi} open and worker routing, tids={','.join(tids)}")
else:
    sys.exit("usage: verify.py {prepare|trace} EVIDENCE-DIR")
