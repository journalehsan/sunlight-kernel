#!/usr/bin/env python3
"""Copy a stopped QEMU WAV capture, fixing only placeholder RIFF/data sizes.
Preserves the original and all PCM bytes; uses bounded memory for long captures.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path


def digest(path, skip=0):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        stream.seek(skip)
        while block := stream.read(1024 * 1024):
            result.update(block)
    return result.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    assert args.input.resolve() != args.output.resolve() and not args.output.exists()
    size = args.input.stat().st_size
    assert 44 < size <= 0xFFFFFFFF + 8 and (size - 44) % 4 == 0
    with args.input.open('rb') as source:
        header = bytearray(source.read(44))
        assert header[:4] == b'RIFF' and header[8:16] == b'WAVEfmt ' and header[36:40] == b'data'
        assert struct.unpack_from('<I', header, 16)[0] == 16
        old_sizes = [struct.unpack_from('<I', header, n)[0] for n in [4, 40]]
        struct.pack_into('<I', header, 4, size - 8)
        struct.pack_into('<I', header, 40, size - 44)
        with args.output.open('xb') as destination:
            destination.write(header)
            while block := source.read(1024 * 1024):
                destination.write(block)
    assert args.input.stat().st_size == size, 'capture changed; stop QEMU first'
    unchanged = digest(args.input, 44) == digest(args.output, 44)
    assert unchanged
    print(json.dumps(dict(original=args.input.name, original_sha256=digest(args.input),
        original_riff_data_sizes=old_sizes, output=args.output.name,
        output_sha256=digest(args.output), pcm_unchanged=unchanged), indent=2))


if __name__ == '__main__':
    main()
