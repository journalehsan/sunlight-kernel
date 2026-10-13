#!/usr/bin/env python3
"""Inventory a Cargo metadata graph, excluding dev-only edges.

Usage: audit.py METADATA.json TREE.txt OUTPUT.md
Generate metadata with --filter-platform x86_64-unknown-none and tree with
-e normal,build --prefix none --format '{p}|{f}' for the same target.
The tree prunes inactive optional dependencies and dev-edge feature pollution
from metadata. Source references are evidence anchors, not a Rust cfg evaluator.
"""
import json
import re
import sys
from pathlib import Path


def inventory(metadata, active):
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    roles = {}

    def visit(pid, role):
        package = packages[pid]
        if (package["name"], package["version"]) not in active:
            return
        if any("proc-macro" in t["kind"] for t in package["targets"]):
            role = "host"
        if role in roles.setdefault(pid, set()):
            return
        roles[pid].add(role)
        for dep in nodes[pid]["deps"]:
            for edge in dep["dep_kinds"]:
                if edge["kind"] == "dev":
                    continue
                visit(dep["pkg"], "host" if edge["kind"] == "build" else role)

    visit(metadata["resolve"]["root"], "target")
    return packages, nodes, roles


# Reviewed published sources, not simply whether a feature happens to be named std.
UNCONDITIONAL = {
    "hayro", "hayro-interpret", "image", "png", "fdeflate", "flate2",
    "alloc-stdlib", "moxcms", "pic-scale", "pxfm", "sunlight-hayro-prototype",
}
SELECTED = {
    "brotli", "brotli-decompressor", "byteorder-lite", "color", "crc32fast",
    "fearless_simd", "font-types", "hayro-cmap", "hayro-jbig2",
    "hayro-jpeg2000", "hayro-syntax", "kurbo", "linebender_resource_handle",
    "num-traits", "peniko", "polycool", "read-fonts", "rustc-hash", "glifo",
    "simd-adler32", "skrifa", "vello_common", "vello_cpu", "zlib-rs",
    "zune-core", "zune-jpeg",
}
NOTES = {
    "hayro": "No no_std switch; std Rc/RefCell; forces image PNG and Vello std/png",
    "hayro-interpret": "No no_std switch; Mutex/OnceLock/LazyLock; forces syntax std/images/unsafe; moxcms",
    "hayro-syntax": "Defaults off builds natively; images also builds; unsafe implies std and flate2",
    "hayro-cmap": "Defaults off is alloc-only; embed-cmaps forces std/LazyLock/Brotli",
    "hayro-postscript": "Alloc-only scanner; not a PostScript document renderer",
    "hayro-ccitt": "Defaults off builds with native images probe",
    "hayro-jbig2": "Defaults off builds with native images probe; SIMD std enabled by syntax unsafe",
    "hayro-jpeg2000": "Defaults off builds with native images probe; SIMD std enabled by syntax unsafe",
    "vello_cpu": "Defaults off + libm/u8_pipeline type-checks; native SIMD codegen fails",
    "vello_common": "libm alternative; png implies std; native SIMD codegen fails",
    "flate2": "Unconditional extern crate std despite #![no_std]; io Read/Write",
    "zlib-rs": "rust-allocator also disables no_std in this release; C allocator alternative exceeds scope",
    "moxcms": "No no_std flag; std io Cursor/Read, time, SIMD dispatch and Arc",
    "pic-scale": "No no_std flag even with defaults off; alloc/math; optional SIMD flags insufficient",
    "pxfm": "No no_std flag; x86 OnceLock and AVX/FMA runtime detection",
    "image": "No no_std flag; io traits; Hayro selects png even for memory-only output",
    "png": "No no_std flag; io traits and fdeflate/flate2",
    "fdeflate": "No no_std flag; io Seek/Write in encoder",
    "alloc-stdlib": "Implicit std prelude; Box/Vec allocator adapter, no no_std feature",
    "skrifa": "Disable defaults, choose libm upstream; embedded standard fonts avoid OS lookup",
    "read-fonts": "std optional; defaults off upstream",
    "font-types": "std optional; defaults off upstream",
    "siphasher": "std feature is empty; source remains no_std; not a direct std blocker",
    "euclid": "Source remains no_std; std forwarded to num-traits",
    "once_cell": "alloc/race only; no std feature selected",
    "foldhash": "Defaults off; alloc/core hashing (metadata std belongs to inactive dev edges)",
    "sunlight-hayro-prototype": "Host driver fs/Instant/System and unwind guard; intentionally host-only",
}


def main():
    metadata = json.loads(Path(sys.argv[1]).read_text())
    active = {}
    for line in Path(sys.argv[2]).read_text().splitlines():
        match = re.match(r"(\S+) v(\S+).*?\|(.*)", line)
        if match:
            name, version, features = match.groups()
            active.setdefault((name, version), set()).update(
                f for f in features.replace(" (*)", "").split(",") if f
            )
    packages, nodes, roles = inventory(metadata, active)
    lines = ["# Hayro Phase 2 complete dependency compatibility matrix", "",
             "Generated from the resolved Phase 1 host graph with native platform filtering.",
             "Dev-only packages/edges excluded. Host means proc macros/build dependencies;",
             "target+host packages are compiled separately for both domains. Features shown",
             "come from cargo tree normal/build edges. H = unconditional std;",
             "F = selected configuration requires std; A = alloc/core-compatible configuration.",
             "A is source evidence, not blanket native execution certification. Build scripts",
             "run on the host even for rows marked target. Source anchors include cfg/test code",
             "and must be interpreted with the reviewed notes, never as mandatory API counts.", "",
             "| Crate | Domain | Status | Resolved features | Finding | Source anchor |",
             "| --- | --- | --- | --- | --- | --- |"]
    for pid in sorted(roles, key=lambda pid: (packages[pid]["name"], packages[pid]["version"])):
        p = packages[pid]
        name = p["name"]
        features = sorted(active[(p["name"], p["version"])])
        status = "H" if name in UNCONDITIONAL else "F" if name in SELECTED else "A"
        if name in {"zune-core", "zune-jpeg"} and "std" not in features:
            status = "A"
        if roles[pid] == {"host"}:
            status = "host std allowed"
        root = Path(p["manifest_path"]).parent
        anchors = []
        libs = [t for t in p["targets"] if "lib" in t["kind"]]
        if libs:
            path = Path(libs[0]["src_path"])
            for number, line in enumerate(path.read_text().splitlines(), 1):
                if line.strip().startswith(("#![no_std", "#![cfg_attr", "extern crate std", "use std::")) and re.search("no_std|std::|extern crate std", line):
                    anchors.append(f"{path.relative_to(root)}:{number}")
        note = NOTES.get(name, "Host generator/derive tooling" if roles[pid] == {"host"} else "Core/alloc utility; selected std needs upstream feature adjustment" if status == "F" else "Core/alloc utility")
        if any("custom-build" in t["kind"] for t in p["targets"]):
            note += "; host build.rs"
        lines.append(f"| {name} {p['version']} | {'+'.join(sorted(roles[pid]))} | {status} | {', '.join(features) or '—'} | {note} | {', '.join(anchors[:3]) or 'Cargo.toml / implicit prelude'} |")
    lines += ["", f"Total: {len(roles)} packages; "
              f"{sum('target' in r for r in roles.values())} target participants; "
              f"{sum(r == {'host'} for r in roles.values())} host-only; "
              f"{sum(r == {'host', 'target'} for r in roles.values())} dual-domain.", ""]
    Path(sys.argv[3]).write_text("\n".join(lines))


if __name__ == "__main__":
    main()
