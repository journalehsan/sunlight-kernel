"""Rebuild the Qalculate candidate from the retained, hash-pinned host snapshot."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from resolve_bundle import resolve


PLUGINS = [
    "Program/plugins/platforms/libqwayland.so",
    "Program/plugins/wayland-shell-integration/libxdg-shell.so",
]
INTERPRETERS = {"/lib64/ld-linux-x86-64.so.2": "Program/lib/ld-linux-x86-64.so.2"}
PLUGIN_ORIGINS = {
    plugin: Path("/usr/lib/qt6") / Path(plugin).relative_to("Program") for plugin in PLUGINS
}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package(path):
    return subprocess.check_output(["pacman", "-Qoq", "--", str(path)], text=True).strip()


def stage(snapshot, work):
    source = snapshot / "Qalculate.sunapp"
    bundle = work / source.name
    pins = {}
    for line in (snapshot / "SHA256SUMS").read_text().splitlines():
        expected, name = line.split("  ", 1)
        path = Path(name)
        if path.is_absolute() or ".." in path.parts or not path.parts:
            raise ValueError(f"invalid snapshot path: {name}")
        if (source / path).resolve(strict=True) != source / path or digest(source / path) != expected:
            raise ValueError(f"snapshot hash or path mismatch: {name}")
        pins[path.as_posix()] = expected
        destination = bundle / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / path, destination)

    entry = "Program/qalculate-qt"
    raw = snapshot / "build/qalculate-qt"
    historical = bytearray((bundle / entry).read_bytes())
    if historical[9:15] != b"HLNX01":
        raise ValueError("expected the original Helios-stamped snapshot")
    historical[9:15] = bytes(6)
    if historical != raw.read_bytes():
        raise ValueError("original executable differs beyond the historical Helios marker")
    shutil.copy2(raw, bundle / entry)

    origins = dict(PLUGIN_ORIGINS)
    for line in (snapshot / "library-origins.txt").read_text().splitlines():
        name, origin = line.split()
        if Path(name).name != name or ".so" not in name:
            raise ValueError(f"invalid library alias: {name}")
        origins[f"Program/lib/{name}"] = Path(origin)
    origins[INTERPRETERS["/lib64/ld-linux-x86-64.so.2"]] = Path("/usr/lib/ld-linux-x86-64.so.2")
    origins["Resources/fonts/LiberationSans-Regular.ttf"] = Path(
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf")
    for pin in pins:
        if pin.startswith("Resources/qalculate/"):
            origins[pin] = Path("/usr/share/qalculate") / Path(pin).name
    provenance, packages = {}, {}
    for name, origin in sorted(origins.items()):
        if not origin.is_absolute() or not origin.is_relative_to("/usr"):
            raise ValueError(f"invalid package origin: {origin}")
        if digest(origin) != pins[name]:
            raise ValueError(f"installed package no longer matches pinned input: {origin}")
        owner = package(origin)
        if owner not in packages:
            metadata = subprocess.check_output(
                ["pacman", "-Qi", "--", owner], text=True, env={**os.environ, "LC_ALL": "C"},
            )
            fields = dict(line.split(":", 1) for line in metadata.splitlines() if ":" in line)
            licenses = Path("/usr/share/licenses") / owner
            destination = bundle / "Licenses/packages" / owner
            if licenses.is_dir():
                shutil.copytree(licenses, destination)
            packages[owner] = {
                "installed_version": next(value.strip() for key, value in fields.items()
                                          if key.strip() == "Version"),
                "declared_licenses": next(value.split() for key, value in fields.items()
                                          if key.strip() == "Licenses"),
                "package_license_files": sorted(
                    str(path.relative_to(bundle)) for path in destination.rglob("*") if path.is_file()
                ),
            }
        staged = bundle / name
        canonical = origin.resolve().name
        aliases = {}
        if name.startswith("Program/lib/") and origin.is_symlink():
            cursor = origin
            while cursor.is_symlink():
                hop = cursor.readlink()
                if len(hop.parts) != 1 or hop.name in ("", ".", "..") or cursor.name in aliases:
                    raise ValueError(f"unsupported package symlink: {cursor} -> {hop}")
                aliases[cursor.name] = hop.name
                cursor = cursor.parent / hop
            payload = staged.with_name(canonical)
            if payload.exists() and digest(payload) != pins[name]:
                raise ValueError(f"conflicting payload: {payload}")
            shutil.copy2(cursor, payload)
            for alias, target in aliases.items():
                link = staged.with_name(alias)
                if link.is_symlink() and link.readlink() == Path(target):
                    continue
                if link.exists() and digest(link) != pins[name]:
                    raise ValueError(f"conflicting alias: {link}")
                link.unlink(missing_ok=True)
                link.symlink_to(target)
        provenance[name] = {
            "source": str(origin), "package": owner, "sha256": pins[name], "aliases": aliases,
        }
    provenance[entry] = {
        "sha256": digest(bundle / entry),
        "source": "Qalculate/qalculate-qt@04a765857db5ac9a3589340a53e25c92d693ab0a",
    }
    (bundle / "Licenses/provenance.json").write_text(json.dumps(
        {"files": provenance, "packages": packages}, indent=2, sort_keys=True) + "\n")
    graph = resolve(bundle, "/Applications/Qalculate.sunapp", ["Program/lib"], PLUGINS, INTERPRETERS)
    extra = sorted(pins.keys() - origins.keys() - {entry, "Manifest.toml", "Program/qt.conf"}
                   - {name for name in pins if name.startswith(("Licenses/", "Resources/icon."))})
    if extra:
        raise ValueError(f"staged files without a recorded origin: {extra}")
    graph["bundle_payload_bytes"] = sum(
        path.stat().st_size for path in bundle.rglob("*") if not path.is_symlink() and path.is_file()
    )
    (work / "dependency-graph.json").write_text(json.dumps(graph, indent=2, sort_keys=True) + "\n")
    if graph["status"] != "PASS":
        raise ValueError(f"dependency validation failed: {graph['errors'] or graph['unresolved']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("output", type=Path, help="new directory for the candidate and graph")
    args = parser.parse_args()
    output = args.output.absolute()
    if output.exists():
        parser.error(f"output already exists: {output}")
    try:
        with tempfile.TemporaryDirectory(dir=output.parent) as directory:
            work = Path(directory) / "candidate"
            work.mkdir()
            stage(args.snapshot.resolve(strict=True), work)
            work.rename(output)
    except (KeyError, OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
