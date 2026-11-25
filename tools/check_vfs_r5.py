"""Check transaction identity, event order and resource disposition in R5 serial logs."""
import json
import re
import sys
from pathlib import Path


def check(path):
    serial = path.read_text(errors="replace").splitlines()

    def once(pattern):
        hits = [(n, match) for n, line in enumerate(serial, 1)
                if (match := re.search(pattern, line))]
        assert len(hits) == 1, (path, pattern, len(hits))
        return hits[0]

    def ordered(events):
        lines = dict(events)
        assert list(lines.values()) == sorted(set(lines.values())), (path, lines)
        return lines

    reused, match = once(r"\[VFS-R5\] reused slot=(\d+) old_child=(\d+) old_retain=(\d+) child=(\d+) retain=(\d+) epoch=(\d+) rejected=1$")
    slot, old, old_id, child, request, epoch = map(int, match.groups())
    assert old != child and old_id != request
    def receipt(pid, req):
        line, match = once(rf"\[VFS-R5\] receipt child={pid} incarnation=(\d+) vm=(\d+) retain={req} service=(\d+) service_incarnation=(\d+) service_vm=(\d+) epoch={epoch}$")
        keys = ("incarnation", "vm", "service", "service_incarnation", "service_vm")
        return line, dict(zip(keys, map(int, match.groups())))

    def release(pid, req):
        return once(rf"\[VFS-R5\] release child={pid} retain={req} epoch={epoch}$")[0]

    old_receipt, old_identity = receipt(old, old_id)
    new_receipt, new_identity = receipt(child, request)
    assert all(old_identity[key] == new_identity[key] for key in ("service", "service_incarnation", "service_vm"))
    assert not any(re.search(r"KERNEL PANIC|panicked at", line) for line in serial)
    parked = lambda pid, req: once(rf"\[VFS-L1C-1\] parked parent=1 child={pid} retain={req} epoch={epoch}$")[0]
    disposed = lambda pid, req, owner: once(rf"\[VFS-L1C-2B\] dispose child={pid} retain={req} owner={owner}$")[0]
    event = lambda label: once(rf"\[VFS-L1C-2\] {label} child={old}$")[0]
    reap, reaped = once(rf"process_reaped pid={old} name='vfs-l1c-private' user_frames=(\d+) page_tables=(\d+) swap_blocks=(\d+) ")
    published = ordered([
        ("parked", parked(old, old_id)), ("receipt", old_receipt), ("binding", event("binding")),
        ("committed", event("committed")), ("enqueue", event("enqueue")),
        ("rejected", once(rf"\[VFS-R5\] published child={old} retain={old_id} slot={slot} rejected=1$")[0]),
        ("disposed", disposed(old, old_id, "scheduler")),
    ])
    dispatch = event("first dispatch selected")
    waited = once(r"\[VFS-R5\] init consumed published status once$")[0]
    assert published["rejected"] < dispatch < min(reap, waited)
    assert waited < reused
    old_release = release(old, old_id)
    assert dispatch < old_release
    teardown, resources = once(rf"\[VFS-R4\] teardown child={child} descriptors=(\d+) frames=(\d+) tables=(\d+) swap=(\d+)$")
    cancelled = ordered([
        ("parked", parked(child, request)), ("receipt", new_receipt), ("reused", reused),
        ("held", once(rf"\[VFS-R4\] B held service=\d+ child={child} retain={request} epoch={epoch}$")[0]),
        ("teardown", teardown),
        ("heap_drop", once(rf"\[VFS-R4\] dropped child={child} process_box=1 fd_table_box=1$")[0]),
        ("disposed", disposed(child, request, "reclaimed")),
        ("reclaimed", once(rf"\[VFS-R4\] C reclaimed child={child} retain={request} slot=\d+ pid_reserved=1 continuation=0$")[0]),
        ("rejected", once(rf"\[VFS-R5\] reclaimed child={child} retain={request} rejected=1$")[0]),
        ("compensation", once(rf"\[VFS-R4\] D late child={child} retain={request} slot=\d+ compensation=1 continuation=0$")[0]),
        ("release", release(child, request)),
        ("settled", once(rf"\[VFS-L1C-1\] settled child={child} retain={request} epoch={epoch}$")[0]),
    ])
    once(r"\[VFS-R5\] init consumed observer status once$")
    assert not any(re.search(rf"\[VFS-L1C-2\] (binding|committed|enqueue|first dispatch selected) child={child}$", line)
                   or f"process_reaped pid={child} " in line for line in serial[cancelled["parked"] - 1:])
    return dict(serial=path.name, slot=slot, epoch=epoch,
                published=dict(child=old, request=old_id, identity=old_identity,
                               lines=dict(published, dispatch=dispatch, reap=reap, wait=waited, release=old_release),
                               resources=dict(zip(("frames", "tables", "swap"), map(int, reaped.groups())))),
                cancelled=dict(child=child, request=request, identity=new_identity, lines=cancelled,
                               resources=dict(zip(("descriptors", "frames", "tables", "swap"), map(int, resources.groups())))))


if __name__ == "__main__":
    print(json.dumps([check(Path(arg)) for arg in sys.argv[1:]], indent=2))
