#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
import re
import shutil
import sys
import subprocess
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from desktop_tty_ux_gate import Guest
from orphan_evidence import validate_orphans
from allocator_evidence import run_host_allocator, validate_allocator


def validate_wait_lifecycle(serial, variants, *, trace_allocations=False):
    events = list(re.finditer(
        r"\[WAIT-LIFE\] ([\w-]+) parent=(\d+:\d+) child=(\d+):(\d+) slot=(\d+) [^\n]*",
        serial,
    ))
    children = {}
    for variant in variants.values():
        pid = variant["pid"]
        matches = [event for event in events if int(event[3]) == pid]
        published, = [event for event in matches if event[1] == "published"]
        exited, = [event for event in matches if event[1] == "exit-published"]
        reaped, = [event for event in matches if event[1] == "wait-reap"]
        cleanup, = [event for event in matches if event[1] == "cleanup-completed"]
        assert all(event.groups()[1:] == published.groups()[1:] for event in matches)
        assert published.start() < exited.start() < reaped.start()
        assert exited.start() < cleanup.start()
        assert "status=Some(0)" in exited[0] and "status=Some(0)" in reaped[0]
        assert "wait=Consumed" in reaped[0]
        if cleanup.start() < reaped.start():
            assert "wait=Pending" in cleanup[0] and "retained=true" in cleanup[0]
        for release in (event for event in matches if event[1] == "slot-release"):
            assert release.start() > max(reaped.start(), cleanup.start())
        children[pid] = (published, reaped, cleanup)

    if trace_allocations:
        return validate_wait_allocations(serial, children)

    reused = []
    for stale in re.finditer(r"\[WAIT-REGRESSION\] old=(\d+) new=(\d+) stale=Err\(Failed\)", serial):
        old, new = map(int, stale.groups())
        previous, reaped, cleanup = children[old]
        replacement, _, _ = children[new]
        if previous[5] != replacement[5]:
            continue
        assert previous[4] != replacement[4] and previous[2] == replacement[2]
        assert max(reaped.start(), cleanup.start()) < replacement.start() < stale.start()
        reused.append(dict(slot=int(previous[5]), old=old, new=new))
    assert reused, "no stale lookup after observed reuse by another probe"
    return dict(reuse=reused, cleanup_before_wait=[
        pid for pid, (_, reaped, cleanup) in children.items() if cleanup.start() < reaped.start()
    ])


def validate_wait_allocations(serial, children):
    allocations = {}
    for event in re.finditer(
        r'\[WAIT-ALLOC\] sequence=(\d+) slot=(\d+) child=(\d+):(\d+) parent=(\d+:\d+) name="([^"\n]*)"',
        serial,
    ):
        pid = int(event[3])
        assert pid not in allocations, f"duplicate allocation PID {pid}"
        allocations[pid] = event

    stale_checks = {}
    for check in re.finditer(r"\[WAIT-REGRESSION\] old=(\d+) new=(\d+) stale=([^\r\n]+)", serial):
        old, new = map(int, check.groups()[:2])
        assert check[3] == "Err(Failed)", check[0]
        assert old != new and old in children and new in children, check[0]
        original, reaped, _ = children[old]
        replacement, replacement_reaped, _ = children[new]
        assert original[2] == replacement[2] and original[4] != replacement[4], check[0]
        assert reaped.start() < replacement.start() < replacement_reaped.start() < check.start(), check[0]
        assert (old, new) not in stale_checks, check[0]
        stale_checks[old, new] = check
    ordered_children = sorted(children, key=lambda pid: children[pid][0].start())
    assert set(stale_checks) == {
        (old, replacement) for index, replacement in enumerate(ordered_children)
        for old in ordered_children[:index]
    }, "missing stale wait checks"

    reuse = []
    slot_relationships = []
    cleanup_before_wait = []
    for pid, (published, reaped, cleanup) in children.items():
        allocation = allocations[pid]
        identity = f"{allocation[3]}:{allocation[4]}"
        assert (allocation[2], identity, allocation[5]) == (published[5], f"{published[3]}:{published[4]}", published[2])
        assert allocation.start() < published.start()
        if cleanup.start() < reaped.start():
            cleanup_before_wait.append(pid)
        releases = list(re.finditer(
            rf"\[WAIT-LIFE\] slot-release parent={published[2]} child={identity} slot={published[5]} [^\n]* next=Some\((\d+)\)",
            serial,
        ))
        assert len(releases) <= 1, identity
        if releases:
            release, = releases
            occupant_pid = int(release[1])
            occupant = allocations[occupant_pid]
            assert max(reaped.start(), cleanup.start()) < release.start() < occupant.start()
            assert occupant[2] == allocation[2] and occupant[4] != allocation[4]
        for (old, replacement_pid), check in stale_checks.items():
            if old != pid:
                continue
            replacement = allocations[replacement_pid]
            slot_relationships.append(dict(old=pid, replacement=replacement_pid,
                                           same_slot=allocation[2] == replacement[2]))
            lookup = list(re.compile(
                rf"\[WAIT-LIFE\] lookup parent={published[2]} child={pid} match=None cpu=\d+",
            ).finditer(serial, allocations[replacement_pid].start(), check.start()))
            assert lookup, check[0]
            if releases and occupant.end() < lookup[-1].start():
                reuse.append(dict(
                    slot=int(allocation[2]), old=pid, old_identity=identity,
                    occupant=occupant_pid, occupant_identity=f"{occupant[3]}:{occupant[4]}",
                    occupant_parent=occupant[5], occupant_name=occupant[6],
                    replacement=replacement_pid, replacement_identity=f"{replacement[3]}:{replacement[4]}",
                    occupant_is_replacement=occupant_pid == replacement_pid,
                ))
    assert stale_checks and reuse, "no matching parent lookup after a traced slot reuse"
    return dict(reuse=reuse, cleanup_before_wait=cleanup_before_wait, slot_relationships=slot_relationships,
                allocations=len(allocations))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("accelerator", choices=("tcg", "kvm"))
    parser.add_argument("--saved", action="store_true", help="validate the existing serial log")
    parser.add_argument("--handoff", metavar="RUN")
    parser.add_argument("--relocation", metavar="RUN")
    parser.add_argument("--waitpid", metavar="RUN")
    parser.add_argument("--shared", metavar="RUN")
    parser.add_argument("--real-shared", metavar="RUN")
    parser.add_argument("--dependency-graph", metavar="RUN")
    parser.add_argument("--slot-reuse", metavar="RUN")
    parser.add_argument("--lifecycle", metavar="RUN")
    parser.add_argument("--tls", metavar="RUN")
    parser.add_argument("--tls-shared", metavar="RUN")
    parser.add_argument("--mmap", metavar="RUN")
    parser.add_argument("--allocator", metavar="RUN")
    parser.add_argument("--malloc", metavar="RUN")
    parser.add_argument("--c-startup", metavar="RUN")
    parser.add_argument("--c-dso", metavar="RUN")
    parser.add_argument("--resolver-negative", action="store_true")
    parser.add_argument("--replay", action="store_true")
    args = parser.parse_args()
    if args.c_dso:
        if args.c_startup:
            parser.error("--c-dso already includes --c-startup controls")
        args.c_startup = args.c_dso
    if args.c_startup:
        if args.malloc:
            parser.error("--c-startup already includes --malloc controls")
        args.malloc = args.c_startup
    if args.malloc:
        if args.allocator:
            parser.error("--malloc already includes --allocator controls")
        args.allocator = args.malloc
    if args.allocator:
        if args.mmap:
            parser.error("--allocator already includes --mmap controls")
        args.mmap = args.allocator
    if args.mmap:
        args.tls_shared = args.mmap
        args.resolver_negative = True
    if args.resolver_negative and not args.tls_shared:
        parser.error("--resolver-negative requires --tls-shared")
    args.tls = args.tls_shared or args.tls
    args.lifecycle = args.tls or args.lifecycle
    args.dependency_graph = args.lifecycle or args.slot_reuse or args.dependency_graph
    args.shared = args.dependency_graph or args.real_shared or args.shared
    if args.waitpid or args.shared:
        args.relocation = args.waitpid or args.shared
    accelerator = args.accelerator
    root = Path("target/helios-dynamic-elf")
    handoff = args.handoff or args.relocation
    if args.c_dso:
        root = Path("target/helios-runtime-c-dso") / args.c_dso
    elif args.c_startup:
        root = Path("target/helios-runtime-c-startup") / args.c_startup
    elif args.malloc:
        root = Path("target/helios-runtime-malloc") / args.malloc
    elif args.allocator:
        root = Path("target/helios-runtime-allocator") / args.allocator
    elif args.mmap:
        root = Path("target/helios-runtime-mmap") / args.mmap
    elif args.tls_shared:
        root = Path("target/helios-dynamic-elf-tls-shared") / args.tls_shared
    elif args.tls:
        root = Path("target/helios-dynamic-elf-tls-bss") / args.tls
    elif args.lifecycle:
        root = Path("target/helios-process-lifecycle-complete") / args.lifecycle
    elif args.slot_reuse:
        root = Path("target/helios-dynamic-elf-slot-reuse") / args.slot_reuse
    elif args.dependency_graph:
        root = Path("target/helios-dynamic-elf-dependency-graph") / args.dependency_graph
    elif args.real_shared:
        root = Path("target/helios-dynamic-elf-real-shared") / args.real_shared
    elif args.shared:
        root = Path("target/helios-dynamic-elf-shared-library") / args.shared
    elif args.waitpid:
        root = Path("target/helios-dynamic-elf-waitpid") / args.waitpid
    elif args.relocation:
        root = Path("target/helios-dynamic-elf-relocation") / args.relocation
    elif args.handoff:
        root = Path("target/helios-dynamic-elf-handoff") / args.handoff
    out = root / (accelerator + "-replay" if args.replay or
                  (not handoff and accelerator == "tcg") else accelerator)
    out.mkdir(parents=True, exist_ok=True)
    reports = ["build.log", "bundle-contents.txt", "manifests.txt", "SHA256SUMS"]
    if args.real_shared or args.dependency_graph:
        reports.extend(("toolchain.txt", "registry.txt"))
    if args.tls:
        reports.extend(("toolchain.txt", "tls-object.dis", "tls.dis", "tls.json",
                        "tls-init-object.dis", "tls-init.dis", "tls-init.json"))
    for name in reports:
        shutil.copyfile(root / name, out / name)
    for report in root.glob("*.readelf"):
        shutil.copyfile(report, out / report.name)
    result = {"accelerator": accelerator, "vcpus": 2, "variants": {}}
    if handoff:
        with (root / "guest.iso").open("rb") as image:
            result["iso_sha256"] = hashlib.file_digest(image, "sha256").hexdigest()
        shutil.copyfile(root / "interpreter-identity.txt", out / "interpreter-identity.txt")
        if args.replay:
            initial = json.loads((root / accelerator / "result.json").read_text())
            assert initial["iso_sha256"] == result["iso_sha256"], "replay image differs"
    guest = None
    try:
        if accelerator == "kvm":
            try:
                os.close(os.open("/dev/kvm", os.O_RDWR))
            except OSError as error:
                result["unavailable"] = (
                    "NOT RUN — /dev/kvm unavailable" if not Path("/dev/kvm").exists()
                    else f"NOT RUN — {error}"
                )
                print(result["unavailable"])
                return
        if not args.saved:
            if args.mmap:
                with (out / "hash-verification.log").open("w") as log:
                    subprocess.run(["sha256sum", "-c", str(root / "SHA256SUMS")],
                                   stdout=log, stderr=log, check=True)
            if args.tls:
                with (out / "host-elf.log").open("w") as log:
                    subprocess.run(["cargo", "test", "-p", "sunlight-elf", "--target",
                                    "x86_64-unknown-linux-gnu"], stdout=log, stderr=log, check=True)
                with (out / "allocator.log").open("w") as log:
                    binary = out / "pid-tests"
                    subprocess.run(["rustc", "--edition=2021", "--test", "kernel/src/process/pid.rs",
                                    "-o", str(binary)], stdout=log, stderr=log, check=True)
                    subprocess.run([str(binary)], stdout=log, stderr=log, check=True)
                binary.unlink()
            if args.mmap:
                with (out / "host-mm2a.log").open("w") as log:
                    subprocess.run(["cargo", "test", "-p", "sunlight-kernel", "--lib",
                                    "--target", "x86_64-unknown-linux-gnu"],
                                   stdout=log, stderr=log, check=True)
            if args.allocator:
                run_host_allocator(out)
            if args.malloc:
                run_host_allocator(out, "bounded-malloc")
            if args.c_startup:
                run_host_allocator(out, "c-startup")
            guest = Guest(root / "guest.iso", out, accel=accelerator, offline=bool(args.mmap),
                          qmp=not args.mmap)
            try:
                guest.wait("[DYNAMIC-ELF] done")
                if args.tls_shared:
                    completed = (out / "serial.log").read_text(errors="replace")
                    for pid in re.findall(r"\[ORPHAN\] replacement variant=tls-shared child=(\d+)", completed):
                        guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                    if args.mmap:
                        for pid in re.findall(r"\[ANON-MMAP(?:-FAULT-WAIT)?\] marker=\w pid=(\d+) waitpid=Ok\((?:0|42)\)", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                    if args.allocator:
                        for pid in re.findall(r"\[PAGE-ALLOC-WAIT\] marker=\w pid=(\d+) waitpid=Ok\(0\)", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                        for pid in re.findall(r"\[ORPHAN\] replacement variant=page-allocator child=(\d+)", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                    if args.malloc:
                        for pid in re.findall(r"\[MALL-ALLOC-WAIT\] marker=\w pid=(\d+) ", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                        for pid in re.findall(r"\[MALL-ALLOC-WAIT\] marker=Y .* owner=(\d+)", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                        for pid in re.findall(r"\[ORPHAN\] replacement variant=malloc child=(\d+)", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                    if args.c_startup:
                        for pid in re.findall(r"\[C-STARTUP-WAIT\] mode=\w pid=(\d+) ", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                        for pid in re.findall(r"\[ORPHAN\] replacement variant=c-startup child=(\d+)", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
                    if args.c_dso:
                        for pid in re.findall(r"\[C-DSO-WAIT\] attempt=\d+ pid=(\d+) ", completed):
                            guest.wait(f"process_resource_cleanup_summary pid={pid} ")
            except RuntimeError:
                if not args.relocation:
                    raise
        serial = (out / "serial.log").read_text(errors="replace")
        if args.lifecycle:
            serial = serial.split("[ORPHAN] begin", 1)[0] + (
                "\n[DYNAMIC-ELF] done\n" if "[DYNAMIC-ELF] done" in serial else ""
            )
        if not args.relocation:
            assert "[DYNAMIC-ELF] done" in serial
        if handoff:
            rollback = re.findall(
                r"\[HELIOS-HANDOFF\] rollback verified case=(\w+) pid=(\d+) free=(\d+) tables=(\d+) owned=(\d+)",
                serial,
            )
            result["rollback"] = {
                case: dict(pid=int(pid), free=int(free), tables=int(tables), owned=int(owned))
                for case, pid, free, tables, owned in rollback
            }
            assert len(rollback) == 9 and set(result["rollback"]) == {
                "missing_interpreter", "rejected_path", "wrong_machine",
                "malformed_interpreter", "application_mapping", "interpreter_mapping",
                "page_table_allocation", "initial_stack", "auxv",
            }
            assert "interpreter validation: WrongMachine" in serial
            assert "interpreter validation: Truncated" in serial
            if args.relocation:
                result["relocation_rollback"] = dict(
                    (case, dict(pid=int(pid), free=int(free), tables=int(tables), owned=int(owned)))
                    for case, pid, free, tables, owned in re.findall(
                        r"\[HELIOS-RELOC\] rollback verified case=(\w+) pid=(\d+) free=(\d+) tables=(\d+) owned=(\d+)",
                        serial,
                    )
                )
                assert set(result["relocation_rollback"]) == {"partial_write", "post_relocation_allocation"}
        attempts = re.split(r"\[DYNAMIC-ELF\] launch /Applications/elf-(\w+)\.sunapp", serial)
        observed = dict(zip(attempts[1::2], attempts[2::2]))
        for variant, blocker, marker in (
            ("pie", "ELF type validation", "header rejected: NotStaticExecutable"),
            ("exec", "PT_INTERP policy", "HasDynamicSection"),
        ):
            attempt = observed[variant]
            assert "phase=spawn_entered" in attempt, attempt
            if handoff:
                metadata = re.search(
                    r"pid=(\d+) AT_BASE=(\w+) AT_PHDR=(\w+) AT_PHNUM=(\d+) "
                    r"AT_ENTRY=(\w+) interpreter_entry=(\w+)", attempt,
                )
                marker = "[HELIOS-HANDOFF] interpreter entry verified; application entry not called"
                failure = re.search(r"rejected stage=(.*?) pid=", attempt)
                result["variants"][variant] = {
                    "first_failure": failure[1] if failure else None,
                    "interpreter_started": marker in attempt,
                }
                assert metadata and marker in attempt, attempt
                pid = int(metadata[1])
                assert f"[DYNAMIC-ELF] pid={pid} waitpid=Ok(0)" in attempt, attempt
                assert re.search(rf"process_mark_finished pid={pid} .*code=0", attempt)
                if guest is not None:
                    guest.wait(f"[SCHED] process_resource_cleanup_summary pid={pid} ")
                cleanup = re.search(
                    rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+) swap=(\d+)",
                    (out / "serial.log").read_text(errors="replace"),
                )
                assert cleanup and int(cleanup[1]) > 0 and int(cleanup[2]) > 0
                result["variants"][variant].update(zip(
                    ("pid", "AT_BASE", "AT_PHDR", "AT_PHNUM", "AT_ENTRY", "interpreter_entry"),
                    (pid, *metadata.groups()[1:]),
                ))
                result["variants"][variant].update(child_exit_status=0, waitpid="Ok(0)",
                                                  relocation=None, tls=None,
                                                  user_frames_reclaimed=int(cleanup[1]),
                                                  page_tables_reclaimed=int(cleanup[2]))
                continue
            assert marker in attempt and "launch failed: SpawnFailed(" in attempt, attempt
            result["variants"][variant] = {
                "result": "expected diagnostic rejection", "first_failure": blocker,
                "interpreter_started": False, "relocation": None, "tls": None,
                "child_exit_status": None, "waitpid": None,
            }
        static = observed["static"]
        assert "phase=spawn_entered" in static, static
        assert "[DYNAMIC-ELF] probe write OK" in static, static
        match = re.search(r"\[DYNAMIC-ELF\] pid=(\d+) waitpid=Ok\(0\)", static)
        assert match, static
        pid = int(match[1])
        assert f"[SYSCALL] spawn: /Applications/elf-static.sunapp/Program/probe pid={pid} " in static
        assert re.search(rf"process_mark_finished pid={pid} .*code=0", static)
        if guest is not None:
            guest.wait(f"[SCHED] process_resource_cleanup_summary pid={pid} ")
        cleanup = re.search(
            rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+) swap=(\d+)",
            (out / "serial.log").read_text(errors="replace"),
        )
        assert cleanup and int(cleanup[1]) > 0 and int(cleanup[2]) > 0
        result["variants"]["static"] = {
            "pid": pid, "output": "probe write OK",
            "child_exit_status": 0, "waitpid": "Ok(0)",
            "user_frames_reclaimed": int(cleanup[1]), "page_tables_reclaimed": int(cleanup[2]),
        }
        if args.relocation:
            attempt = observed.get("relocation", "")
            result["first_failure"] = None
            checks = (
                ("application-entry handoff", r"pid=(\d+) AT_BASE=\w+ AT_PHDR=\w+ AT_PHNUM=\d+ AT_ENTRY=\w+ interpreter_entry=\w+"),
                ("application-entry handoff", r"\[HELIOS-RELOC\] interpreter verified initial stack; entering application"),
                ("application runtime", r"\[HELIOS-RELOC\] application entry verified relocated pointer"),
            )
            stage = re.search(r"rejected stage=(.*?) pid=", attempt)
            if stage:
                result["first_failure"] = stage[1]
                raise AssertionError(attempt)
            for label, pattern in checks:
                match = re.search(pattern, attempt)
                if not match:
                    result["first_failure"] = label
                    raise AssertionError(attempt)
                if match.lastindex:
                    pid = int(match[1])
            for label, pattern in (
                ("relocation-table validation", rf"pid={pid} type=R_X86_64_RELATIVE address=(\w+) value=(\w+)"),
                ("process exit", rf"process_mark_finished pid={pid} .*code=0"),
                ("waitpid", rf"\[DYNAMIC-ELF\] pid={pid} waitpid=Ok\(0\)"),
            ):
                match = re.search(pattern, attempt)
                if not match:
                    result["first_failure"] = label
                    raise AssertionError(attempt)
                if match.lastindex:
                    address, value = match.groups()
            result["first_failure"] = "reclamation"
            if guest is not None:
                guest.wait(f"[SCHED] process_resource_cleanup_summary pid={pid} ")
            cleanup = re.search(
                rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+) swap=(\d+)",
                (out / "serial.log").read_text(errors="replace"),
            )
            assert cleanup and int(cleanup[1]) > 0 and int(cleanup[2]) > 0
            result["first_failure"] = None
            result["variants"]["relocation"] = {
                "pid": pid, "relocation": {"type": "R_X86_64_RELATIVE", "count": 1,
                                          "address": address, "value": value},
                "application_entry": "application entry verified relocated pointer",
                "child_exit_status": 0, "waitpid": "Ok(0)",
                "user_frames_reclaimed": int(cleanup[1]), "page_tables_reclaimed": int(cleanup[2]),
            }
            assert "[DYNAMIC-ELF] done" in serial
            if args.shared:
                result["first_failure"] = "rollback"
                cases = re.findall(
                    r"\[HELIOS-SHARED\] rollback verified case=(\w+) pid=(\d+) free=(\d+) tables=(\d+) owned=(\d+)",
                    serial,
                )
                result["shared_rollback"] = {
                    case: dict(pid=int(pid), free=int(free), tables=int(tables), owned=int(owned))
                    for case, pid, free, tables, owned in cases
                }
                expected_cases = {
                    "needed", "missing_library", "wrong_identity", "malformed_library",
                    "wrong_library_machine", "library_mapping", "dynamic_table", "symbol_table",
                    "jump_slot_write", "shared_stack", "shared_auxv", "process_allocation",
                }
                if args.real_shared or "real" in observed:
                    expected_cases.update({
                        "real_wrong_hash", "real_malformed", "real_mapping", "real_missing_export",
                        "real_duplicate_export", "real_bad_undefined_symbol", "real_symbol_range",
                        "real_relocation_range", "real_unsupported_relocation",
                    })
                assert len(cases) == len(expected_cases) and set(result["shared_rollback"]) == expected_cases
                probes = [("shared", "libhelios-value.so", "helios_fixture_value", "SHARED")]
                if args.real_shared or "real" in observed:
                    probes.append(("real", "libhelios-math.so", "helios_scale", "REAL"))
                for variant, library, symbol, tag in probes:
                    attempt = observed.get(variant, "")
                    if "[DYNAMIC-ELF] launch failed:" in attempt and "phase=spawn_entered" not in attempt:
                        result["first_failure"] = "application entry"
                        result["failure_detail"] = "SunApp launch rejected before kernel spawn"
                        raise AssertionError(attempt)
                    stage = re.search(r"rejected stage=(.*?) pid=", attempt)
                    if stage:
                        result["first_failure"] = stage[1]
                        raise AssertionError(attempt)
                    result["first_failure"] = "trusted library lookup"
                    mapped = re.search(
                        rf"\[HELIOS-SHARED\] pid=(\d+) source=embedded-bundle library=Program/{re.escape(library)} "
                        r"app_base=(\w+) interpreter_base=(\w+) library_base=(\w+)", attempt,
                    )
                    assert mapped, attempt
                    pid = int(mapped[1])
                    result["first_failure"] = "relocation"
                    relocation = re.search(
                        rf"pid={pid} image=Program/probe type=R_X86_64_JUMP_SLOT address=(\w+) "
                        rf"symbol={symbol} resolved=(\w+) addend=0 value=(\w+)", attempt,
                    )
                    assert relocation and relocation[2] == relocation[3], attempt
                    for label, pattern in (
                        ("application entry", r"\[HELIOS-RELOC\] interpreter verified initial stack; entering application"),
                        ("application entry", rf"\[HELIOS-{tag}\] application entered"),
                        ("application runtime", rf"\[HELIOS-{tag}\] result=42"),
                        ("process exit", rf"process_mark_finished pid={pid} .*code=0"),
                        ("waitpid", rf"\[DYNAMIC-ELF\] pid={pid} waitpid=Ok\(0\)"),
                    ):
                        result["first_failure"] = label
                        assert re.search(pattern, attempt), attempt
                    result["first_failure"] = "reclamation"
                    if guest is not None:
                        guest.wait(f"[SCHED] process_resource_cleanup_summary pid={pid} ")
                    serial = (out / "serial.log").read_text(errors="replace")
                    cleanup = re.search(
                        rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+) swap=(\d+)", serial,
                    )
                    assert cleanup and int(cleanup[1]) > 0 and int(cleanup[2]) > 0
                    result["variants"][variant] = dict(
                        pid=pid, application_base=mapped[2], interpreter_base=mapped[3], library_base=mapped[4],
                        relocation_address=relocation[1], resolved_symbol=relocation[2], result=42,
                        child_exit_status=0, waitpid="Ok(0)", user_frames_reclaimed=int(cleanup[1]),
                        page_tables_reclaimed=int(cleanup[2]),
                    )
                if args.dependency_graph or "graph" in observed:
                    validate_graph(observed.get("graph", ""), serial, result)
                result["first_failure"] = None
            if args.waitpid or args.shared:
                serial = (out / "serial.log").read_text(errors="replace")
                (out / "wait-lifecycle.log").write_text("\n".join(
                    line for line in serial.splitlines()
                    if any(tag in line for tag in (
                        "[WAIT-LIFE]", "[WAIT-REGRESSION]", "[WAIT-ALLOC]", "process_mark_finished",
                        "process_resource_cleanup_summary",
                    ))
                ) + "\n")
                traced = bool(args.slot_reuse or "[WAIT-ALLOC]" in serial)
                result["first_failure"] = "stale PID validation" if traced else "wait lifecycle regression"
                result["lifecycle"] = validate_wait_lifecycle(
                    serial, result["variants"], trace_allocations=traced,
                )
                for variant in ("real", "graph"):
                    if variant in result["variants"] and (args.real_shared or args.dependency_graph):
                        if traced:
                            result["first_failure"] = "process-slot reuse"
                        pid = result["variants"][variant]["pid"]
                        assert any(event["old"] == pid for event in result["lifecycle"]["reuse"]), (
                            f"{variant} probe's slot was not observed in reuse"
                        )
                result["first_failure"] = None
        if args.lifecycle:
            serial = (out / "serial.log").read_text(errors="replace")
            result["first_failure"] = "parent exits before collection"
            validate_orphans(serial, out, result, tls=bool(args.tls), tls_shared=bool(args.tls_shared), mmap=bool(args.mmap),
                             allocator=bool(args.allocator), malloc=bool(args.malloc),
                             c_startup=bool(args.c_startup))
            result["first_failure"] = None
        if args.tls:
            validate_tls(serial, out, result)
        if args.tls_shared:
            from orphan_evidence import TLS_SHARED_OUTPUT, one
            result["first_failure"] = "startup TLS rollback"
            rollback = re.findall(
                r"\[HELIOS-TLS-SHARED\] rollback verified case=startup_tls_(\w+) pid=(\d+) free=(\d+) tables=(\d+) owned=(\d+)",
                serial,
            )
            assert len(rollback) == 15 and {label for label, *_ in rollback} == {
                "module", "missing", "duplicate", "malformed", "filesz", "alignment", "unmapped",
                "allocation", "table", "resolver", "relocation", "index", "offset", "stack", "interpreter",
            }
            assert all(owned == "0" for _, _, _, _, owned in rollback)
            result["tls_shared_rollback"] = {label: dict(pid=int(pid), free=int(free), tables=int(tables), owned=int(owned))
                                             for label, pid, free, tables, owned in rollback}
            result["first_failure"] = "shared TLS sequential isolation"
            waits = list(re.finditer(r"\[HELIOS-TLS-SHARED\] sequential pid=(\d+) waitpid=Ok\(0\)", serial))
            assert len(waits) == 3 and len({wait[1] for wait in waits}) == 3
            pids = [wait[1] for wait in waits]
            pids.append(result["parent_before_collection"]["tls-shared"]["child"].split(":")[0])
            pids.extend(row["identity"].split(":")[0] for row in result["replacement_controls"]
                        if row["variant"] == "tls-shared")
            assert len(set(pids)) == 7 and serial.count(TLS_SHARED_OUTPUT) == 7
            children = []
            for pid in pids:
                setup = one(rf"\[HELIOS-TLS-SHARED\] pid={pid} module=1 .* module=2 .*", serial)
                binding = one(rf"\[HELIOS-TLS-SHARED\] pid={pid} hash=verified .*module=2 offset=0 .*", serial, setup.end())
                exited = one(rf"\[WAIT-LIFE\] exit-published .* child={pid}:\d+ .*status=Some\(0\)", serial)
                identity = one(rf"\[WAIT-ALLOC\] sequence=\d+ slot=(\d+) child={pid}:(\d+) ", serial)
                vm = one(rf"\[ORPHAN-VM\] child={pid}:{identity[2]} vm=(\d+) regions=0 root_reclaimed=true owned_frames=0", serial)
                cleanup = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
                assert identity.start() < exited.start() < vm.start() < cleanup.start()
                assert int(cleanup[1]) > 0 and int(cleanup[2]) > 0
                children.append(dict(pid=int(pid), slot=int(identity[1]), incarnation=int(identity[2]),
                                     vm_generation=int(vm[1]), frames=int(cleanup[1]), page_tables=int(cleanup[2]),
                                     owned=0, exit_status=0))
                if pid in pids[:3]:
                    wait = waits[pids.index(pid)]
                    printed = one(re.escape(TLS_SHARED_OUTPUT), serial, binding.end(), exited.start())
                    assert exited.start() < wait.start()
                    assert serial[binding.end():printed.start()].count(
                        "[HELIOS-TLS-SHARED] resolver module=2 offset=0 result=dso-block"
                    ) == 2
            assert len({row["vm_generation"] for row in children}) == 7
            result["tls_shared_children"] = children
            result["first_failure"] = None
        if args.resolver_negative:
            validate_resolver_negative(serial, result)
        if args.mmap:
            result["first_failure"] = "anonymous mapping probe"
            validate_mmap(serial, out, result)
            result["first_failure"] = None
        if args.allocator:
            result["first_failure"] = "page allocator probe"
            validate_allocator(serial, out, result)
            result["first_failure"] = None
        if args.malloc:
            from malloc_evidence import validate_malloc
            result["first_failure"] = "bounded malloc probe"
            validate_allocator(serial, out, result, malloc=True)
            validate_malloc(serial, out, result)
            result["first_failure"] = None
        if args.c_startup:
            from startup_evidence import validate_startup
            result["first_failure"] = "C startup probe"
            validate_startup(serial, out, result)
            result["first_failure"] = None
        if args.c_dso:
            from c_dso_evidence import validate_c_dso
            result["first_failure"] = "C utility DSO"
            validate_c_dso(serial, out, result)
            result["first_failure"] = None
    finally:
        if args.relocation and (out / "serial.log").exists() and "relocation" not in result["variants"]:
            serial = (out / "serial.log").read_text(errors="replace")
            attempt = serial.partition("[DYNAMIC-ELF] launch /Applications/elf-relocation.sunapp")[2]
            stage = re.search(r"rejected stage=(.*?) pid=", attempt)
            if stage:
                result["first_failure"] = stage[1]
            elif re.search(r"\[DYNAMIC-ELF\] pid=\d+ waitpid=Err\(", attempt):
                result["first_failure"] = "waitpid"
        (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        if guest is not None:
            guest.close()


def validate_mmap(serial, out, result):
    from orphan_evidence import one

    rows = []
    start = one(r"\[ORPHAN\] done", serial).end()
    for marker in "123":
        wait = one(rf"\[ANON-MMAP\] marker={marker} pid=(\d+) waitpid=Ok\(0\)", serial, start)
        pid = wait[1]
        window = serial[start:wait.start()]
        for message in (
            "mmap invalid requests rejected",
            "length=4096 mapping=slot-1 flags=0x22 fd=-1 offset=0 initial=RW zero=1",
            "mprotect invalid requests rejected",
            "mprotect result=0 requested=R readback=match",
            f"mprotect result=0 requested=RW marker={marker}",
            "munmap result=0 repeat=0 stale-mprotect=-12 invalid=-22",
            "exit=0",
        ):
            one(re.escape(f"[ANON-MMAP] {message}"), window)
        trace = re.findall(
            rf"\[ANON-MMAP-VM\] pid={pid} op=(\w+) ok=(true|false) before=(.*) after=(.*)",
            window,
        )
        for _, ok, before, after in trace:
            if ok == "false":
                assert before == after, (pid, before, after)
        transitions = [(op, before, after) for op, ok, before, after in trace if ok == "true"]
        assert [op for op, *_ in transitions] == ["map", "protect", "protect", "unmap", "unmap"]
        rw = "Some((true, true, false))"
        read = "Some((true, false, false))"
        assert transitions[0][2].endswith(f"{rw}, None)")
        assert transitions[1][1].endswith(f"{rw}, {rw})")
        assert transitions[1][2].endswith(f"{read}, {read})")
        assert transitions[2][2].endswith(f"{rw}, {rw})")
        assert transitions[3][2].endswith("None, None)")
        assert transitions[4][1] == transitions[4][2]
        assert transitions[0][1] == transitions[3][2], transitions
        identity = one(rf"\[WAIT-ALLOC\] sequence=\d+ slot=(\d+) child={pid}:(\d+) ", window)
        cleanup = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
        one(rf"\[ORPHAN-VM\] child={pid}:{identity[2]} vm=\d+ regions=0 root_reclaimed=true owned_frames=0", serial)
        rows.append(dict(marker=marker, pid=int(pid), slot=int(identity[1]),
                         incarnation=int(identity[2]), frames=int(cleanup[1]),
                         tables=int(cleanup[2]), waitpid="Ok(0)", trace=trace))
        start = wait.end()
    assert len({row["pid"] for row in rows}) == 3
    for message in (
        "[KFP2A3-TABLE] partial one/two-table allocation rollback PASS",
        "[MM-2A] injected mmap/SHM allocation rollback: OK",
        "[ANON-MMAP-FAULT] eager-publication stale-reservation rollback=pass root=reclaimed owned=0",
    ):
        one(re.escape(message), serial)
    result["mm2a_eager_allocation_rollback"] = "PASS"
    result["mm2a_eager_publication_rollback"] = "PASS"

    live = []
    for marker in "4567":
        wait = one(rf"\[ANON-MMAP\] marker={marker} pid=(\d+) waitpid=Ok\(0\)", serial, start)
        pid = wait[1]
        window = serial[start:wait.start()]
        identity = one(
            rf'\[WAIT-ALLOC\] sequence=\d+ slot=(\d+) child={pid}:(\d+) parent=(\d+:\d+) ',
            window,
        )
        cleanup = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
        one(rf"\[ORPHAN-VM\] child={pid}:{identity[2]} vm=\d+ regions=0 root_reclaimed=true "
            r"owned_frames=0 stack_live=false cleanup_pending=false", serial)
        trace = re.findall(
            rf"\[ANON-MMAP-VM\] pid={pid} op=(\w+) ok=(true|false) before=(.*) after=(.*)",
            window,
        )
        for _, ok, before, after in trace:
            if ok == "false":
                assert before == after, (pid, before, after)
        assert trace[0][0:2] == ("map", "true")
        assert trace[0][2].endswith("None, None)")
        assert trace[0][2].split(", ")[1] == rows[0]["trace"][0][2].split(", ")[1]
        spans = re.findall(rf"\[ANON-MMAP-SPAN\] pid={pid} requested=(\d+) effective=(\d+)", window)
        if marker == "6":
            one(re.escape("[ANON-MMAP-LIVE] exhausted errno=-12 existing=valid stale=-12 exit=live"), window)
            assert trace[-2][0:2] == ("map", "false")
            assert trace[-1][0:2] == ("protect", "false")
            assert spans == [("4096", "4096")] * len(spans)
            assert trace[-2][2].split(", ")[1] == "256"
            assert int(cleanup[1]) == rows[0]["frames"] + len(spans)
        else:
            one(re.escape(f"[ANON-MMAP-LIVE] zero=both marker={marker} exit=live"), window)
            assert spans == [("8192" if marker == "4" else "4097", "8192"), ("4097", "8192")]
            assert trace[1][3].endswith(f"{read}, {read})")
            assert trace[2][3].endswith(f"{rw}, {rw})")
            assert trace[-1][0:2] == ("protect", "false")
            assert trace[-1][3].endswith("None, None)")
            assert int(cleanup[1]) == rows[0]["frames"] + 2
        assert int(cleanup[2]) == rows[0]["tables"] + 2
        live.append(dict(marker=marker, pid=int(pid), parent=identity[3],
                         slot=int(identity[1]), incarnation=int(identity[2]),
                         frames=int(cleanup[1]), tables=int(cleanup[2]),
                         waitpid="Ok(0)", spans=spans, trace=trace))
        start = wait.end()
    assert len({row["pid"] for row in rows + live}) == 7
    assert live[0]["slot"] == live[1]["slot"]
    assert live[2]["slot"] == live[3]["slot"]
    faults = {}
    for marker, status, message in (
        ("P", 0, "protect rc=-12 markers=P,P writable=both unmapped=both"),
        ("S", 42, "setup rc=-22 markers=S,S exit=42 live=both"),
        ("Z", 0, "replacement zero=both old-marker=absent"),
    ):
        wait = one(rf"\[ANON-MMAP-FAULT-WAIT\] marker={marker} pid=(\d+) waitpid=Ok\({status}\)", serial, start)
        pid = wait[1]
        window = serial[start:wait.start()]
        identity = one(
            rf'\[WAIT-ALLOC\] sequence=\d+ slot=(\d+) child=({pid}:\d+) parent=(\d+:\d+) ',
            window,
        )
        slot, child, parent = identity.groups()
        exit_event = one(rf"\[WAIT-LIFE\] exit-published parent={parent} child={child} slot={slot} .*status=Some\({status}\)", window)
        reap = one(rf"\[WAIT-LIFE\] wait-reap parent={parent} child={child} slot={slot} .*wait=Consumed status=Some\({status}\)", window)
        guest = one(re.escape(f"[ANON-MMAP-FAULT-GUEST] {message}"), window)
        assert identity.start() < guest.start() < exit_event.start() < reap.start()
        one(rf"\[ORPHAN-VM\] child={child} vm=\d+ regions=0 root_reclaimed=true owned_frames=0 stack_live=false cleanup_pending=false", serial)
        cleanup = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
        one(rf"\[ANON-MMAP-SPAN\] pid={pid} requested=8192 effective=8192", window)
        trace = re.findall(
            rf"\[ANON-MMAP-VM\] pid={pid} op=(\w+) ok=(true|false) before=(.*) after=(.*)", window,
        )
        assert trace[0][2].endswith("None, None)")
        expected_ops = {
            "P": [("map", "true"), ("protect", "false"), ("unmap", "true"), ("protect", "false")],
            "S": [("map", "true"), ("map", "false"), ("protect", "false")],
            "Z": [("map", "true")],
        }
        assert [(op, ok) for op, ok, _, _ in trace] == expected_ops[marker]
        for _, ok, before, after in trace:
            if ok == "false":
                assert before == after
        if marker == "P":
            one(rf"\[ANON-MMAP-FAULT\] pid={pid} protect leaf=2 applied=1 restored=1 ptes=RW,RW metadata=RW errno=-12", window)
            assert trace[1][2].endswith(f"{rw}, {rw})")
            assert trace[0][2] == trace[2][3]
        if marker == "S":
            one(rf"\[ANON-MMAP-FAULT\] pid={pid} setup scratch publication cancelled requested=4097 effective=8192", window)
            assert trace[-1][3].endswith("None, None)")
        frames, tables = map(int, cleanup.groups())
        extra = 0 if marker == "P" else 2
        assert frames == rows[0]["frames"] + extra
        assert tables == rows[0]["tables"] + extra
        faults[marker] = dict(child=child, parent=parent, slot=int(slot), waitpid=f"Ok({status})",
                              frames=frames, tables=tables, trace=trace, guest=message)
        start = wait.end()
    assert faults["S"]["slot"] == faults["Z"]["slot"]
    assert faults["S"]["child"] != faults["Z"]["child"]
    result["anonymous_mmap_faults"] = faults
    result["anonymous_mmap_live"] = live
    result["anonymous_mmap"] = rows
    (out / "anon-mmap.log").write_text("\n".join(
        line for line in serial.splitlines() if line.startswith("[ANON-MMAP")
    ) + "\n")


def validate_resolver_negative(serial, result):
    from orphan_evidence import one

    result["first_failure"] = "direct guest TLS resolver rejection"
    rows = []
    start = one(r"\[ORPHAN\] done", serial).end()
    for case in ("m", "o", "x", "p"):
        waited = one(
            rf"\[HELIOS-TLS-NEG\] case={case} pid=(\d+) wait=Ok\(1\) again=Err\(Failed\)",
            serial, start,
        )
        pid = waited[1]
        published = one(r"\[WAIT-LIFE\] published .* child=(\d+):(\d+) ", serial,
                        start, waited.start())
        assert published[1] == pid
        binding = one(rf"\[HELIOS-TLS-SHARED\] pid={pid} hash=verified .*", serial, start, waited.start())
        called = one(rf"\[HELIOS-TLS-NEG\] call={case}", serial, binding.end(), waited.start())
        assert published.start() < called.start()
        exited = one(rf"\[WAIT-LIFE\] exit-published .* child={pid}:{published[2]} .*status=Some\(1\)",
                     serial, called.end(), waited.start())
        vm = one(rf"\[ORPHAN-VM\] child={pid}:{published[2]} vm=(\d+) regions=0 root_reclaimed=true owned_frames=0 stack_live=false cleanup_pending=false",
                 serial, exited.end())
        cleanup = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial, vm.end())
        assert int(cleanup[1]) > 0 and int(cleanup[2]) > 0
        assert "result=dso-block" not in serial[called.end():exited.start()]
        rows.append(dict(case=case, pid=int(pid), incarnation=int(published[2]),
                         vm_generation=int(vm[1]), frames=int(cleanup[1]),
                         page_tables=int(cleanup[2]), owned_frames=0, exit_status=1,
                         repeat_wait="Err(Failed)"))
        start = waited.end()
    assert len({row["pid"] for row in rows}) == 4
    result["resolver_negative"] = rows
    result["first_failure"] = None


def validate_tls(serial, out, result):
    from orphan_evidence import TLS_INIT_OUTPUT, TLS_OUTPUT, one

    result["first_failure"] = "initialized TLS control"
    control = one(r"\[HELIOS-TLS-INIT\] control pid=(\d+) waitpid=Ok\(0\)", serial)
    setup = one(rf"\[HELIOS-TLS\] setup pid={control[1]} tp=(0x[0-9a-f]+) image=(0x[0-9a-f]+) "
                r"filesz=4 memsz=4 model=local-exec", serial, end=control.start())
    assert int(setup[1], 16) - int(setup[2], 16) == 4
    ready = one(r"\[HELIOS-TLS-INIT\] interpreter initial=17", serial, setup.end(), control.start())
    printed = one(re.escape(TLS_INIT_OUTPUT) + r"\n", serial, ready.end(), control.start())
    one(rf"\[WAIT-LIFE\] exit-published .* child={control[1]}:\d+ .*status=Some\(0\)",
        serial, printed.end(), control.start())
    result["tls_initialized_control"] = dict(pid=int(control[1]), exit_status=0, waitpid="Ok(0)")

    result["first_failure"] = "rollback"
    rollback = re.findall(
        r"\[HELIOS-TLS\] rollback verified case=(\w+) pid=(\d+) free=(\d+) tables=(\d+) owned=(\d+)", serial,
    )
    assert len(rollback) == 10 and {case for case, *_ in rollback} == {
        "tls_malformed", "tls_alignment", "tls_unmapped", "tls_filesz", "tls_overflow",
        "tls_allocation", "tls_stack", "tls_interpreter", "tls_process", "tls_missing_tail",
    }
    assert all(owned == "0" for _, _, _, _, owned in rollback)
    malformed = one(r"\[HELIOS-TLS\] rollback verified case=tls_malformed ", serial)
    missing = one(r"\[HELIOS-TLS\] rollback verified case=tls_missing_tail pid=(\d+)", serial)
    one(rf"\[HELIOS-HANDOFF\] rejected stage=PT_TLS validation: missing BSS tail pid={missing[1]}\n",
        serial, malformed.end(), missing.start())
    for case in ("stack", "interpreter"):
        one(rf"\[HELIOS-TLS\] setup pid=\d+ [^\n]+ filesz=4 memsz=8 model=local-exec\n(?:(?!\[HELIOS-TLS\]).*\n)*"
            rf"\[HELIOS-TLS\] rollback verified case=tls_{case} ", serial)
    result["tls_rollback"] = dict((case, dict(pid=int(pid), free=int(free), tables=int(tables),
                                            owned=int(owned))) for case, pid, free, tables, owned in rollback)
    result["first_failure"] = "waitpid"
    waits = list(re.finditer(r"\[HELIOS-TLS\] sequential pid=(\d+) waitpid=Ok\(0\)", serial))
    assert len(waits) == 3
    pids = [wait[1] for wait in waits]
    pids += [result["parent_before_collection"]["tls"]["child"].split(":")[0]]
    pids += [row["identity"].split(":")[0] for row in result["replacement_controls"] if row["variant"] == "tls"]
    assert len(set(pids)) == 7
    result["first_failure"] = "application TLS access"
    assert serial.count(TLS_OUTPUT) == len(pids)
    assert serial.count("[HELIOS-TLS] interpreter initial=17") == len(pids)
    result["tls"] = []
    previous = 0
    for index, pid in enumerate(pids):
        result["first_failure"] = "thread-pointer setup"
        begin = waits[index - 1].end() if 0 < index < len(waits) else previous
        setup = one(rf"\[HELIOS-TLS\] setup pid={pid} tp=(0x[0-9a-f]+) image=(0x[0-9a-f]+) filesz=4 memsz=8 model=local-exec",
                    serial, begin)
        assert int(setup[1], 16) - int(setup[2], 16) == 8
        result["first_failure"] = "application TLS access"
        exit_status = one(rf"\[WAIT-LIFE\] exit-published .* child={pid}:\d+ .*status=Some\(0\)", serial)
        assert setup.start() < exit_status.start()
        ready = one(r"\[HELIOS-TLS\] interpreter initial=17", serial, setup.end(), exit_status.start())
        one(re.escape(TLS_OUTPUT), serial, ready.end(), exit_status.start())
        if index < len(waits):
            assert exit_status.start() < waits[index].start()
            previous = waits[index].end()
        result["first_failure"] = "process cleanup"
        cleanup = one(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+)", serial)
        identity = one(rf"\[WAIT-ALLOC\] sequence=\d+ slot=(\d+) child={pid}:(\d+) ", serial)
        assert cleanup.start() > exit_status.start() and int(cleanup[1]) > 0 and int(cleanup[2]) > 0
        vm = one(rf"\[ORPHAN-VM\] child={pid}:{identity[2]} vm=(\d+) regions=0 root_reclaimed=true owned_frames=0", serial)
        assert exit_status.start() < vm.start() < cleanup.start()
        result["tls"].append(dict(pid=int(pid), slot=int(identity[1]), incarnation=int(identity[2]),
                                  vm_generation=int(vm[1]), thread_pointer=setup[1], tls_image=setup[2],
                                  filesz=4, memsz=8, initialized=17, tail_initial=0, tail_reads=[53, 67],
                                  output=TLS_OUTPUT, exit_status=0, frames_reclaimed=int(cleanup[1]),
                                  page_tables_reclaimed=int(cleanup[2]), owned_frames_remaining=0))
    assert len({row["vm_generation"] for row in result["tls"]}) == len(pids)
    result["first_failure"] = "process cleanup"
    result["tls_slot_reuse"] = []
    for earlier, later in zip(result["tls"][:2], result["tls"][1:3]):
        if earlier["slot"] != later["slot"]:
            continue
        one(rf"\[WAIT-LIFE\] slot-release .* child={earlier['pid']}:{earlier['incarnation']} "
            rf"slot={earlier['slot']} .*state=Reaped wait=Consumed .*next=Some\({later['pid']}\)", serial)
        assert earlier["incarnation"] != later["incarnation"]
        result["tls_slot_reuse"].append(dict(slot=earlier["slot"], old=earlier["pid"], new=later["pid"]))
    assert result["tls_slot_reuse"], "no sequential TLS child reused a TLS child's slot"
    result["first_failure"] = None


def validate_graph(attempt, serial, result):
    result["first_failure"] = "rollback"
    cases = re.findall(
        r"\[HELIOS-GRAPH\] rollback verified case=(\w+) pid=(\d+) free=(\d+) tables=(\d+) owned=(\d+)",
        serial,
    )
    expected = {
        "missing_a", "missing_b", "hash_a", "hash_b", "undeclared", "cycle", "duplicate",
        "unresolved", "ambiguous", "path", "needed_offset", "mapping", "leaf_relocation", "page_table", "stack", "process",
    }
    assert len(cases) == len(expected) and {row[0] for row in cases} == {"graph_" + name for name in expected}
    result["graph_rollback"] = {
        case: dict(pid=int(pid), free=int(free), tables=int(tables), owned=int(owned))
        for case, pid, free, tables, owned in cases
    }
    result["first_failure"] = "process entry"
    stage = re.search(r"rejected stage=(.*?) pid=", attempt)
    if stage:
        result["first_failure"] = stage[1]
        raise AssertionError(attempt)
    result["first_failure"] = "dependency graph traversal"
    edges = re.findall(
        r"\[HELIOS-GRAPH\] pid=(\d+) owner=(\S+) needed=(\S+) hash=verified order=(\d+)"
        r" base=(\w+) app_base=(\w+) interpreter_base=(\w+)", attempt,
    )
    app, lib_a, lib_b = "Program/probe", "Program/libhelios_a.so", "Program/libhelios_b.so"
    assert [(edge[1], edge[2], edge[3]) for edge in edges] == [(app, lib_a, "0"), (lib_a, lib_b, "1")], attempt
    assert edges[0][0] == edges[1][0] and edges[0][4] != edges[1][4], attempt
    pid = int(edges[0][0])
    result["first_failure"] = "relocation"
    slots = re.findall(
        rf"\[HELIOS-GRAPH\] pid={pid} image=(\S+) type=R_X86_64_JUMP_SLOT address=(\w+)"
        r" symbol=(\S+) provider=(\S+) resolved=(\w+) addend=0 value=(\w+)", attempt,
    )
    assert [(slot[0], slot[2], slot[3]) for slot in slots] == [
        (app, "helios_graph_a", lib_a), (lib_a, "helios_graph_b", lib_b),
        (lib_b, "helios_graph_multiply", lib_b),
    ], attempt
    assert all(slot[4] == slot[5] for slot in slots), attempt
    for label, pattern in (
        ("process entry", r"\[HELIOS-GRAPH\] application entered"),
        ("process entry", r"\[HELIOS-GRAPH\] result=42"),
        ("process exit", rf"process_mark_finished pid={pid} .*code=0"),
        ("waitpid", rf"\[DYNAMIC-ELF\] pid={pid} waitpid=Ok\(0\)"),
    ):
        result["first_failure"] = label
        assert re.search(pattern, attempt), attempt
    result["first_failure"] = "reclamation"
    cleanup = re.search(rf"process_resource_cleanup_summary pid={pid} frames=(\d+) pts=(\d+) swap=(\d+)", serial)
    assert cleanup and int(cleanup[1]) > 0 and int(cleanup[2]) > 0
    result["variants"]["graph"] = dict(
        pid=pid, edges=edges, relocations=slots, result=42, child_exit_status=0, waitpid="Ok(0)",
        user_frames_reclaimed=int(cleanup[1]), page_tables_reclaimed=int(cleanup[2]),
    )


if __name__ == "__main__":
    main()
