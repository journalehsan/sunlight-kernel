"""Run the read probe and BTop-08 controls in the hardware TTY."""
from pathlib import Path
import runpy
import sys

root = Path(__file__).resolve().parents[2]
output = root / "target/btop-10"
source = (root / "tools/btop-07/replay.py").read_text()


def replace(before, after):
    global source
    assert source.count(before) == 1, before
    source = source.replace(before, after)


replace("extra = [", '''extra = ["-trace", "enable=virtio_blk_handle_read",
         "-trace", "enable=virtio_blk_rw_complete",
         "-trace", "enable=virtio_blk_req_complete", ''')
replace("    if args.probe:\n", '''    attached = guest.request("query-block")
    (out / "attached.json").write_text(json.dumps(attached, indent=2))
    fixture = [entry for entry in attached if entry["device"] == "fixture"]
    if args.disk:
        assert len(fixture) == 1 and fixture[0]["inserted"]["ro"]
        assert fixture[0]["inserted"]["file"] == str(disk)
    else:
        assert not fixture

    def mark(phase):
        blockstats(phase)
        with (out / "phases.jsonl").open("a") as stream:
            print(json.dumps(dict(phase=phase, monotonic_ns=time.monotonic_ns(),
                                  serial=guest.serial.stat().st_size,
                                  qemu=(out / "qemu.log").stat().st_size)), file=stream)

    mark("before-idle")
    time.sleep(3)
    mark("after-idle")
    run_command("/bin/storage-probe", "0" if args.disk else "3", timeout=45)
    mark("after-first")
    run_command("/bin/storage-probe", "0" if args.disk else "3", timeout=45)
    mark("after-repeat")
    if args.disk:
        assert guest.read().count("READ10 verified bytes=65536 EOF=1") == 2
    else:
        assert guest.read().count("READ10 open errno=2") == 2
    if args.probe:
''')
generated = output / "replay.generated.py"
replace('Path(__file__).resolve().parents[2]', f"Path({str(root)!r})")
replace('str(Path(__file__).with_name("fat-reference.py"))',
        repr(str(root / "tools/btop-07/fat-reference.py")))
generated.write_text(source)
if "--disk" in sys.argv:
    index = sys.argv.index("--disk")
    if Path(sys.argv[index + 1]).absolute() != output / "disk.img":
        raise SystemExit("use the disposable BTop-10 image")
    sys.argv.append("--readonly")
runpy.run_path(str(generated), run_name="__main__")
