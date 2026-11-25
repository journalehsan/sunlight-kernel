import hashlib
import json
import re
import sys
from pathlib import Path

variant, artifact, report = sys.argv[1:]
inspection = Path(report).read_text()
segments = re.findall(r"^\s*TLS\s+(0x\w+)\s+(0x\w+)\s+0x\w+\s+(0x\w+)\s+(0x\w+)\s+R\s+(0x\w+)$",
                      inspection, re.MULTILINE)
if len(segments) != 1:
    raise SystemExit("expected exactly one read-only PT_TLS")
offset, address, filesz, memsz, align = (int(word, 16) for word in segments[0])
expected = {"value": 0, "tail": 4} if variant == "tls" else {"value": 0}
symbols = {name: int(value, 16) for value, name in re.findall(
    r"^\s*\d+:\s+(\w+)\s+4\s+TLS\s+LOCAL\s+DEFAULT\s+\d+\s+(\w+)$", inspection, re.MULTILINE)}
data = Path(artifact).read_bytes()
initial = data[offset:offset + filesz]
if ((filesz, memsz, align) != (4, 4 * len(expected), 4) or symbols != expected
        or initial != b"\x11\0\0\0" or "There are no relocations in this file." not in inspection):
    raise SystemExit(f"unexpected {variant} PT_TLS layout or relocations")
print(json.dumps(dict(p_offset=offset, p_vaddr=address, p_filesz=filesz, p_memsz=memsz,
                      p_align=align, tls_model="local-exec", symbol_offsets=symbols,
                      final_relocations=[], init_sha256=hashlib.sha256(initial).hexdigest(),
                      artifact_sha256=hashlib.sha256(data).hexdigest()), indent=2))
