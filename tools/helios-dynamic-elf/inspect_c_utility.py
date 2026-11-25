"""Check the toolchain-produced utility ELF inventory before embedding it."""
import re
import sys
from pathlib import Path

report = Path(sys.argv[1]).read_text()
expected = {"helios_memset", "helios_memcpy", "helios_strlen", "helios_write", "helios_exit"}

def require(condition, item):
    if not condition:
        raise SystemExit(f"utility ELF rejected: {item}")

require(re.search(r"Type:\s+DYN\b", report) and
        re.search(r"Machine:\s+Advanced Micro Devices X86-64", report), "type or machine")
require(not re.search(r"^\s+(TLS|INTERP)\s", report, re.M), "PT_TLS or PT_INTERP")
require(re.findall(r"^\s*0x[0-9a-f]+\s+\((\w+)\)", report.split("Dynamic section", 1)[1].split("\n\n", 1)[0], re.M)
        == ["SONAME", "HASH", "STRTAB", "SYMTAB", "STRSZ", "SYMENT", "NULL"],
        "dynamic tags")
require("Library soname: [libhelios-util.so]" in report, "SONAME")
require("There are no relocations in this file." in report, "relocations")
require("No version information found in this file." in report, "symbol versions")
require(not re.search(r"\.(?:preinit|init|fini|ctors|dtors)(?:[_.\s])", report),
        "initialization or finalization sections")
require(not re.search(r"^ *\[ *[0-9]+\] +\.eh_frame\s+\S+\s+\S+\s+\S+\s+0*[1-9a-f][0-9a-f]*\b", report, re.M),
        "nonempty unwind section")
symbols = report.split("Symbol table '.dynsym'", 1)[1].split("\n\n", 1)[0]
rows = [line.split() for line in symbols.splitlines() if re.match(r"\s*\d+:\s", line)]
require(len(rows) == len(expected) + 1 and
        {row[7] for row in rows[1:] if row[3:6] == ["FUNC", "GLOBAL", "DEFAULT"]
         and row[6].isdigit()} == expected, "exported or undefined symbols")
