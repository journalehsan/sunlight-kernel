#!/usr/bin/env python3
"""Prepare an isolated DN 1.51 diagnostic bundle, never install or launch it.

Based on Dos Navigator by RIT Research Labs.
The archive is an upstream prebuilt binary, NOT a SunlightOS source build.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
URL = "https://www.ritlabs.com/download/files3/dn/dn151.zip"
SHA256 = "f6dee3904945a2f390f8576a5d18d83446f4554e43d6f0a3c75f8aa5b570d7d9"
REVISION = "b268678fad8a9ccb3fd6a9463e1e341ac73985ec"
ARTIFACTS = ROOT / "target/dos-navigator"

MANIFEST = r'''[bundle]
format = 1

[application]
id = "org.sunlight.chronos.dn151.diagnostic"
name = "DOS Navigator 1.51 (Compatibility Diagnostic)"
version = "1.51.0"
kind = "terminal-window"
icon = "Resources/icon.tga"

[runtime]
type = "chronos"

[entry]
executable = "C:\\DN.COM"
working_directory = "C:\\"
arguments = []

[chronos]
cpu = "80286"
memory_kb = 640
code_page = 437

[storage]
c_base = ["Program"]
c_overlay = "persistent"
d_scope = "none"
t_drive = "temporary"

[permissions]
documents = "none"
clipboard = false
printing = false
serial = false
network = false

[window]
default_width = 1000
default_height = 720
resizable = true
'''


def prepare(archive: Path) -> Path:
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if digest != SHA256:
        raise ValueError(f"archive checksum mismatch: {digest}")
    bundle = ARTIFACTS / "package/DosNavigatorDiagnostic.sunapp"
    # Validate every member before writing anything. Do not trust ZIP names
    # even though this particular distribution is pinned by hash.
    with zipfile.ZipFile(archive) as source:
        for member in source.infolist():
            path = PurePosixPath(member.filename)
            if (path.is_absolute() or ".." in path.parts or "\\" in member.filename
                    or ":" in member.filename or (member.external_attr >> 16) & 0o170000 == 0o120000):
                raise ValueError(f"unsafe archive member: {member.filename}")
        if bundle.exists():
            raise FileExistsError(f"diagnostic bundle already exists: {bundle}")
        program = bundle / "Program"
        program.mkdir(parents=True)
        source.extractall(program)
    (bundle / "Licenses").mkdir()
    shutil.copyfile(program / "README.TXT", bundle / "Licenses/RIT-Labs.txt")
    (bundle / "Resources").mkdir()
    shutil.copyfile(ROOT / "ChronosDosShell.sunapp/Resources/icon.tga", bundle / "Resources/icon.tga")
    (bundle / "Manifest.toml").write_text(MANIFEST)
    (bundle / "README.md").write_text(
        "# DOS Navigator compatibility diagnostic\n\n"
        "Based on Dos Navigator by RIT Research Labs.\n\n"
        "BLOCKED: this bundle does not yet run a usable file manager in Chronos. "
        "Do not install it as a working application. It grants no Documents access. "
        "Uses the shared Chronos runtime; no runtime is bundled.\n\n"
        "Contains the unmodified RIT Labs DN 1.51 prebuilt distribution. "
        "No source build or bit-for-bit correspondence to the source revision is claimed. "
        "See provenance.json and Licenses/RIT-Labs.txt.\n"
    )
    files = {
        path.relative_to(bundle).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(bundle.rglob("*")) if path.is_file()
    }
    (bundle / "provenance.json").write_text(json.dumps({
        "status": "BLOCKED", "source_built": False,
        "version": "1.51", "archive_url": URL, "archive_sha256": SHA256,
        "source_url": "https://github.com/maximmasiutin/Dos-Navigator",
        "source_revision": REVISION, "source_binary_correspondence": "unverified",
        "files_sha256": files,
    }, indent=2, sort_keys=True) + "\n")
    return bundle


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="use an already downloaded dn151.zip")
    args = parser.parse_args()
    archive = args.archive
    if archive is None:
        ARTIFACTS.mkdir(parents=True, exist_ok=True)
        archive = ARTIFACTS / "dn151.zip"
        if not archive.exists():
            with urllib.request.urlopen(URL, timeout=30) as response:
                data = response.read(4 * 1024 * 1024 + 1)
            if len(data) > 4 * 1024 * 1024:
                raise ValueError("archive exceeds download limit")
            if hashlib.sha256(data).hexdigest() != SHA256:
                raise ValueError("download checksum mismatch")
            archive.write_bytes(data)
    print(prepare(archive))


if __name__ == "__main__":
    main()
