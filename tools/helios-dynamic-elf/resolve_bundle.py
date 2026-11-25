"""Validate the declared static ELF closure of a Helios SunApp."""

import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import posixpath
import re
import subprocess
import tomllib


def inspect(path):
    elf = subprocess.check_output(
        ["readelf", "-WhldVn", "--", str(path)], text=True,
        stderr=subprocess.PIPE, env={**os.environ, "LC_ALL": "C"},
    )
    header = dict(re.findall(r"^  (Class|Data|OS/ABI|ABI Version|Machine|Type):\s*(.+)$", elf, re.M))
    if (header.get("Class") != "ELF64"
            or header.get("Machine") != "Advanced Micro Devices X86-64"
            or header.get("Data") != "2's complement, little endian"
            or header.get("OS/ABI") not in ("UNIX - System V", "UNIX - GNU")
            or header.get("ABI Version") != "0"
            or header.get("Type", "").split()[0:1] not in (["DYN"], ["EXEC"])):
        raise ValueError("expected a Linux-compatible x86-64 ELF executable or DSO")

    def tags(name):
        return re.findall(r"\(" + name + r"\).*?\[(.*?)\]", elf)

    definitions, requirements = set(), defaultdict(set)
    section = provider = None
    for line in elf.splitlines():
        if line.startswith("Version "):
            section = line.split()[1]
            provider = None
        if match := re.search(r"^\s*(?:0x)?[0-9a-f]+:.*\bFile: (\S+)\s+Cnt:", line):
            provider = match[1]
        if match := re.search(r"^\s*0x[0-9a-f]+:(?:\s+Rev:.*|\s+)\bName: (\S+)", line):
            if section == "definition":
                definitions.add(match[1])
            elif section == "needs" and provider:
                requirements[provider].add(match[1])
    interpreter = re.findall(r"Requesting program interpreter: (.*?)\]", elf)
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return {
        "abi": header,
        "abi_notes": re.findall(r"^.*(?:OS: Linux, ABI:|x86 ISA needed:).*$", elf, re.M),
        "interpreter": next(iter(interpreter), None),
        "needed": tags("NEEDED"),
        "soname": next(iter(tags("SONAME")), None),
        "rpath": tags("RPATH"),
        "runpath": tags("RUNPATH"),
        "version_definitions": sorted(definitions),
        "version_requirements": {name: sorted(items) for name, items in requirements.items()},
        "load_segments": re.findall(r"^\s+LOAD\s+(.+)$", elf, re.M),
        "sha256": digest,
    }


def resolve(bundle, install, library_dirs, plugins, interpreters):
    bundle = bundle.resolve(strict=True)
    install = posixpath.normpath(install)
    if not install.startswith("/") or install.startswith("//") or install == "/":
        raise ValueError("expected an absolute bundle installation path")

    def relative(path):
        value = posixpath.normpath(path)
        if value.startswith(("/", "../")) or value == "..":
            raise ValueError(f"not a bundle path: {path}")
        return value

    def locate(path):
        path = relative(path)
        pending = path.split("/")
        resolved = Path()
        hops = {}
        while pending:
            part = pending.pop(0)
            if part in ("", "."):
                continue
            if part == "..":
                if resolved == Path():
                    raise ValueError(f"symlink escapes bundle: {path}")
                resolved = resolved.parent
                continue
            candidate = resolved / part
            if (bundle / candidate).is_symlink():
                target = (bundle / candidate).readlink().as_posix()
                if target.startswith("/") or len(hops) >= 40 or str(candidate) in hops:
                    raise ValueError(f"absolute or cyclic symlink: {candidate} -> {target}")
                hops[str(candidate)] = target
                pending = target.split("/") + pending
            else:
                resolved = candidate
        return str(resolved), hops

    manifest_path, _ = locate("Manifest.toml")
    manifest_bytes = (bundle / manifest_path).read_bytes()
    if len(manifest_bytes) > 8192:
        raise ValueError("manifest exceeds Helios size limit")
    manifest = tomllib.loads(manifest_bytes.decode("utf-8"))
    if (type(manifest["bundle"]["format"]) is not int or manifest["bundle"]["format"] != 1
            or manifest["runtime"]["type"] != "helios"
            or manifest["application"]["kind"] not in ("gui", "terminal")
            or not re.fullmatch(r"[A-Za-z0-9_.-]{1,80}", manifest["application"]["id"])
            or set(manifest) != {"bundle", "runtime", "application", "entry"}):
        raise ValueError("expected a format 1 Helios SunApp manifest")
    entry = manifest["entry"]["executable"]
    if not re.fullmatch(r"Program/[A-Za-z0-9_.-]{1,64}", entry) or entry.endswith(("/.", "/..")):
        raise ValueError("expected a direct Program/ executable")
    dirs = [relative(path) for path in library_dirs]
    roots = {"entry": entry, "plugins": [relative(path) for path in plugins]}
    report = {
        "install_path": install, "library_dirs": dirs, "roots": roots,
        "interpreter_bindings": interpreters, "objects": {}, "edges": [],
        "symlinks": {}, "cycles": [], "unresolved": [], "errors": [],
    }
    visited, active, sonames = set(), [], {}

    def expand(values, path):
        result = []
        origin = posixpath.dirname(f"{install}/{path}")
        for value in (item for row in values for item in row.split(":")):
            expanded = re.sub(r"\$\{ORIGIN\}|\$ORIGIN\b", lambda _: origin, value)
            if not expanded.startswith("/") or "$" in expanded:
                raise ValueError(f"{path}: unsupported ELF search path {value!r}")
            expanded = posixpath.normpath(expanded)
            local = (posixpath.relpath(expanded, install)
                     if expanded == install or expanded.startswith(install + "/") else None)
            result.append({"raw": value, "expanded": expanded, "bundle_dir": local})
        return result

    def visit(path, inherited=(), shared=False):
        canonical, hops = locate(path)
        report["symlinks"].update(hops)
        objects = report["objects"]
        if canonical not in objects:
            objects[canonical] = inspect(bundle / canonical)
        item = objects[canonical]
        if shared and (not item["abi"]["Type"].startswith("DYN")
                       or (item["interpreter"] and not item["soname"])):
            raise ValueError(f"{path}: expected a DSO")
        if soname := item["soname"]:
            previous = sonames.setdefault(soname, canonical)
            if previous != canonical:
                raise ValueError(f"conflicting SONAME {soname}: {previous}, {canonical}")
        if canonical in active:
            cycle = active[active.index(canonical):] + [canonical]
            if cycle not in report["cycles"]:
                report["cycles"].append(cycle)
            return item
        if path in visited:
            return item
        visited.add(path)
        active.append(canonical)
        try:
            rpath = expand(item["rpath"], path)
            runpath = expand(item["runpath"], path)
            local = lambda rows: [row["bundle_dir"] for row in rows if row["bundle_dir"] is not None]
            legacy = tuple(dict.fromkeys([*local(rpath), *inherited])) if not runpath else inherited
            search = list(dict.fromkeys([*(legacy if not runpath else ()), *dirs, *local(runpath)]))
            item.setdefault("search_paths", {})[path] = {"rpath": rpath, "runpath": runpath,
                                                        "effective_bundle_dirs": search}
            dependencies = [("needed", name) for name in item["needed"]]
            if path == entry and item["interpreter"]:
                dependencies.insert(0, ("interpreter", item["interpreter"]))
            unknown = set(item["version_requirements"]) - set(item["needed"])
            if unknown:
                raise ValueError(f"{path}: versions reference undeclared providers: {sorted(unknown)}")
            for kind, name in dependencies:
                edge = {"from": path, "kind": kind, "name": name, "to": None}
                report["edges"].append(edge)
                if kind == "interpreter":
                    choices = [interpreters[name]] if name in interpreters else []
                else:
                    if "/" in name or "$" in name or name in ("", ".", ".."):
                        raise ValueError(f"{path}: unsupported DT_NEEDED {name!r}")
                    choices = [posixpath.join(directory, name) for directory in search]
                candidates = []
                for choice in choices:
                    target, hops = locate(choice)
                    if (bundle / target).is_file():
                        candidates.append((relative(choice), target))
                    elif hops:
                        raise ValueError(f"{choice}: broken library symlink")
                if not candidates:
                    report["unresolved"].append(edge.copy())
                    continue
                if len({target for _, target in candidates}) > 1:
                    raise ValueError(f"{path}: conflicting providers for {name}: {candidates}")
                alias, target = candidates[0]
                edge.update(to=target, via=alias)
                definition = visit(alias, legacy, shared=True)
                expected = posixpath.basename(name)
                if definition["soname"] not in (None, expected):
                    raise ValueError(f"{alias}: expected SONAME {expected}, got {definition['soname']}")
                missing = set(item["version_requirements"].get(expected, [])) - set(
                    definition["version_definitions"])
                if missing:
                    raise ValueError(f"{path}: {alias} lacks versions {', '.join(sorted(missing))}")
        finally:
            active.pop()
        return item

    for path in [entry, *roots["plugins"]]:
        try:
            item = visit(path, shared=path != entry)
            if path == entry and not item["interpreter"]:
                raise ValueError("entry has no PT_INTERP")
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            report["errors"].append(f"{path}: {error}")
    report["resolved_dependencies"] = len({edge["to"] for edge in report["edges"] if edge["to"]})
    report["status"] = "FAIL" if report["errors"] or report["unresolved"] else "PASS"
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--install", required=True)
    parser.add_argument("--library-dir", action="append", default=[])
    parser.add_argument("--plugin", action="append", default=[])
    parser.add_argument("--interpreter", action="append", default=[], metavar="GUEST_PATH=BUNDLE_PATH")
    args = parser.parse_args()
    try:
        report = resolve(args.bundle, args.install, args.library_dir, args.plugin,
                         dict(binding.split("=", 1) for binding in args.interpreter))
    except (KeyError, OSError, TypeError, ValueError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    if report["status"] != "PASS":
        parser.exit(1, f"{report['errors'] or report['unresolved']} (see {args.output})\n")


if __name__ == "__main__":
    main()
