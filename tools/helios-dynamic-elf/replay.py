#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
import re
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from desktop_tty_ux_gate import Guest


def validate_wait_lifecycle(serial, variants):
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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("accelerator", choices=("tcg", "kvm"))
    parser.add_argument("--saved", action="store_true", help="validate the existing serial log")
    parser.add_argument("--handoff", metavar="RUN")
    parser.add_argument("--relocation", metavar="RUN")
    parser.add_argument("--waitpid", metavar="RUN")
    parser.add_argument("--replay", action="store_true")
    args = parser.parse_args()
    if args.waitpid:
        args.relocation = args.waitpid
    accelerator = args.accelerator
    root = Path("target/helios-dynamic-elf")
    handoff = args.handoff or args.relocation
    if args.waitpid:
        root = Path("target/helios-dynamic-elf-waitpid") / args.waitpid
    elif args.relocation:
        root = Path("target/helios-dynamic-elf-relocation") / args.relocation
    elif args.handoff:
        root = Path("target/helios-dynamic-elf-handoff") / args.handoff
    out = root / (accelerator + "-replay" if args.replay or
                  (not handoff and accelerator == "tcg") else accelerator)
    out.mkdir(parents=True, exist_ok=True)
    for name in ("build.log", "bundle-contents.txt", "manifests.txt", "SHA256SUMS"):
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
                result["unavailable"] = str(error)
                return
        if not args.saved:
            guest = Guest(root / "guest.iso", out, accel=accelerator)
            try:
                guest.wait("[DYNAMIC-ELF] done")
            except RuntimeError:
                if not args.relocation:
                    raise
        serial = (out / "serial.log").read_text(errors="replace")
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
            if args.waitpid:
                serial = (out / "serial.log").read_text(errors="replace")
                (out / "wait-lifecycle.log").write_text("\n".join(
                    line for line in serial.splitlines()
                    if "[WAIT-LIFE]" in line or "[WAIT-REGRESSION]" in line
                ) + "\n")
                result["first_failure"] = "wait lifecycle regression"
                result["lifecycle"] = validate_wait_lifecycle(serial, result["variants"])
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


if __name__ == "__main__":
    main()
