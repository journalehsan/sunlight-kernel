"""Use the PROC-01 payload injector with a separately built kernel image."""
from pathlib import Path
import hashlib
import sys

source = Path("tools/btop-proc-01/image.py").read_text()
base = Path(sys.argv.pop(1))
digest = hashlib.file_digest(base.open("rb"), "sha256").hexdigest()
source = source.replace(
    'target / "btop-22/final/guest-ramfs-trace-config.iso"', repr(str(base))
).replace("source = " + repr(str(base)), "source = Path(" + repr(str(base)) + ")")
source = source.replace(
    "e7575a42b56655c41b578157b8cf4fa9c2e575dd25b8bffff5d90a8bb79eb148", digest
).replace("assert image.find(btop) == 33436780", """offset = image.find(btop)
        assert offset >= 0
        print(f"btop: offset={offset} size={len(btop)} sha256={hashlib.sha256(image[offset:offset+len(btop)]).hexdigest()}")""")
exec(compile(source, "tools/btop-proc-01/image.py", "exec"))
