"""Drive the mixed fixture using the PROC-01 hardware TTY runner."""
from pathlib import Path
import sys

ordered = "--ordered" in sys.argv
if ordered:
    sys.argv.remove("--ordered")
registry = "--registry" in sys.argv
if registry:
    sys.argv.remove("--registry")
source = Path(__file__).resolve().parents[1] / "btop-proc-01/replay.py"
code = source.read_text()
if registry:
    code = code.replace('command("/bin/tty-select " + shell)',
                        'command("/bin/tty-select " + shell + " registry")')
if ordered:
    hook = '        guest.capture("after")'
    extra = '''
        command("/bin/route-scope ordered-read")
        target = match(r"\\[PROC-ORDER\\] checkpoint=spawn parent=\\d+ target=(\\d+)", 20)[1]
        for checkpoint in ("ready", "open", "release", "wait", "read-enter", "read-return"):
            match(rf"\\[PROC-ORDER\\] checkpoint={checkpoint} target={target}\\b.*", 12)
        serial = guest.read()
        reap = re.search(rf"\\[SCHED\\] process_reaped\\s+pid={target}\\b", serial)
        read = re.search(rf"\\[PROC-ORDER\\] checkpoint=read-enter target={target}\\b", serial)
        assert reap and read and reap.start() < read.start(), "reap before read was not established"
        match(r"\\[TTY\\]  exit: /bin/route-scope -> 0", 12)
'''
    code = code.replace("code = source.read_text()", "code = source.read_text()\n"
                        + f"assert {hook!r} in code\n"
                        + f"code = code.replace({hook!r}, {hook + extra!r})")
for old, new in (
    ("spawn: /bin/route-scope pid=", r"[PROC-FIX] path=native-start pid="),
    ("process_reaped pid=", r"[PROC-FIX] path=native-exit pid="),
    (" name='masked-select'", " result=0"),
):
    assert code.count(old) == 1, old
    code = code.replace(old, new.replace("[PROC-FIX]", r"\\[PROC-FIX\\]"))
exec(
    compile(code.replace(
        'exec(compile(code,',
        '''code = code.replace('"3.39 GiB" in dashboard', '"GiB" in dashboard and "btop" in dashboard')
exec(compile(code,''',
    ), str(source), "exec"),
    {"__file__": str(source)},
)
