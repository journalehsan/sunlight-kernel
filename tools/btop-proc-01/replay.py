"""Run the existing TTY replay with the concurrent fixture image."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[2]
source = root / "tools/btop-06/guest.py"
code = source.read_text()
start = code.index('        command("/bin/btop")')
end = code.index('        guest.key("h")', start)
code = code[:start] + '''        shell = match(r"Spawning /bin/sshl \\(pid=(\\d+)\\)")[1]
        command("/bin/tty-select " + shell)
        pid = match(r"spawn: /bin/btop pid=(\\d+)")[1]
        guest.key("delete")
        time.sleep(7)
        guest.capture("stable")
        match(r"spawn: /bin/route-scope pid=(\\d+)", timeout=35)
        time.sleep(5)
        guest.capture("appeared")
        match(r"process_reaped pid=\\d+ name='masked-select'", timeout=55)
        time.sleep(4)
        guest.capture("removed")
''' + code[end:]
code = code.replace('match(r"\\[TTY\\]  exit: /bin/btop -> 0")',
                    'match(r"\\[PROC-FIX\\] path=btop-exit pid=\\d+ result=0")\n'
                    '        match(r"\\[TTY\\]  exit: /bin/tty-select .*-> 0", timeout=90)')
code = code.replace('f"/proc/{pid}/stat",', '')
exec(compile(code, str(source), "exec"), {"__file__": str(source), "__name__": "__main__"})
