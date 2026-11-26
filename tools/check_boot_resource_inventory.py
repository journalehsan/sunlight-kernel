#!/usr/bin/env python3
"""Compare SUNRES01 files and metadata to the hash-matched audited kernel ELF."""
import ctypes
import hashlib
import json
import struct
import sys
from pathlib import Path

audit = json.loads(Path(sys.argv[1]).read_text())["measured"]
baseline = Path(sys.argv[2]).read_bytes()
assert hashlib.sha256(baseline).hexdigest() == audit["artifact"]["sha256"]
archive = Path(sys.argv[3]).read_bytes()
magic, size, count, payload, index_end = struct.unpack_from("<8s4I", archive)
assert magic == b"SUNRES01" and size == len(archive)
records = audit["ramfs"]["entries"]
assert count == len(records)
decode = ctypes.CDLL("liblz4.so.1").LZ4_decompress_safe
decode.argtypes = [ctypes.c_char_p, ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
decode.restype = ctypes.c_int

cursor = 24
block_size = 4096
scratch = ctypes.create_string_buffer(block_size)
logical_bytes = 0
for record in records:
    path_size, uid, gid, mode, directory, reserved, length, index = struct.unpack_from(
        "<IIIHBBII", archive, cursor
    )
    cursor += 24
    path = archive[cursor:cursor + path_size].decode()
    cursor += path_size
    assert (path, uid, gid, mode, directory, reserved, length) == (
        record["path"], record["uid"], record["gid"], int(record["mode"], 8),
        int(record["kind"] == "directory"), 0, record["bytes"],
    ), record["path"]
    logical_bytes += length
    for block, offset in enumerate(range(0, length, block_size)):
        start, stored, _ = struct.unpack_from("<III", archive, index + 12 * block)
        expected = min(block_size, length - offset)
        assert payload <= start and start + stored <= size and 0 < stored <= expected
        data = archive[start:start + stored]
        if stored < expected:
            assert decode(data, scratch, stored, expected) == expected
            data = scratch.raw[:expected]
        baseline_offset = record["offset"] + offset
        assert data == baseline[baseline_offset:baseline_offset + expected], path
assert cursor == index_end
print(json.dumps({
    "entries_matched": count,
    "logical_bytes_matched": logical_bytes,
    "baseline_sha256": audit["artifact"]["sha256"],
    "module_sha256": hashlib.sha256(archive).hexdigest(),
}))
