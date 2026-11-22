#!/usr/bin/env python3
"""Check or rebuild the normal DOS Navigator SunApp from the pinned DN archive."""

import argparse
import hashlib
from pathlib import Path
import shutil
import tomllib
import zipfile

from dos_navigator_prepare import ROOT, SHA256

BUNDLE = ROOT / "DosNavigator.sunapp"
INVENTORY = ROOT / "docs/audit/chronos-dn/dn-23/program.sha256"


def verify_program(program, expected):
    files = {p.relative_to(program).as_posix(): p for p in program.rglob("*") if p.is_file()}
    if files.keys() != expected.keys():
        raise ValueError("Program inventory does not match the authentic distribution")
    for name, path in files.items():
        if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != expected[name]:
            raise ValueError(f"Program checksum mismatch: {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="verify against the original dn151.zip")
    parser.add_argument("--output", type=Path, help="write a fresh DosNavigator.sunapp (requires --archive)")
    args = parser.parse_args()
    expected = dict((name, digest) for digest, name in
                    (line.split("  ", 1) for line in INVENTORY.read_text().splitlines()))
    bundle = BUNDLE
    if args.output and not args.archive:
        parser.error("--output requires --archive")
    if args.archive:
        if hashlib.sha256(args.archive.read_bytes()).hexdigest() != SHA256:
            raise ValueError("wrong DN 1.51 archive checksum")
        with zipfile.ZipFile(args.archive) as source:
            names = [i.filename for i in source.infolist() if not i.is_dir()]
            if len(names) != len(expected) or set(names) != expected.keys():
                raise ValueError("wrong DN 1.51 archive inventory")
            contents = {name: source.read(name) for name in names}
        for name, data in contents.items():
            if hashlib.sha256(data).hexdigest() != expected[name]:
                raise ValueError(f"archive member checksum mismatch: {name}")
        if args.output:
            bundle = args.output
            if bundle.name != BUNDLE.name:
                parser.error(f"--output must end in {BUNDLE.name}")
            bundle.mkdir(parents=True)
            for name in ("Manifest.toml", "README.md"):
                shutil.copyfile(BUNDLE / name, bundle / name)
            shutil.copytree(BUNDLE / "Resources", bundle / "Resources")
            for name, data in contents.items():
                path = bundle / "Program" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
    verify_program(bundle / "Program", expected)
    with (bundle / "Manifest.toml").open("rb") as stream:
        manifest = tomllib.load(stream)
    if (manifest["bundle"]["format"] != 1 or manifest["runtime"]["type"] != "chronos"
            or manifest["application"]["id"] != "org.sunlight.chronos.dos-navigator"
            or manifest["entry"] != {"executable": r"C:\DN.COM", "working_directory": "C:\\", "arguments": []}
            or manifest["chronos"]["cpu"] != "386"
            or manifest["storage"]["c_base"] != ["Program"]
            or manifest["permissions"] != dict(documents="none", clipboard=False, printing=False,
                                               serial=False, network=False)):
        raise ValueError("unexpected DN SunApp manifest")
    if not (bundle / manifest["application"]["icon"]).is_file():
        raise ValueError("missing bundle icon")
    ramfs = (ROOT / "sunlight-fs/src/ramfs.rs").read_text()
    files = sorted(p.relative_to(bundle).as_posix() for p in bundle.rglob("*") if p.is_file())
    if any(f'"/Applications/{BUNDLE.name}/{name}"' not in ramfs for name in files):
        raise ValueError("missing static install entry")
    print(f"PASS: {len(expected)} byte-identical DOS files; manifest, icon and static install paths verified")


if __name__ == "__main__":
    main()
