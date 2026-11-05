#!/usr/bin/env python3
"""Check changed Rust spans against the preserved exact starting tree."""
import difflib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
for name in ('kernel/build.rs', 'kernel/src/arch/x86_64/syscall.rs',
             'kernel/src/main.rs', 'kernel/src/process/fd_table.rs',
             'kernel/src/sched/mod.rs', 'kernel/src/sched/t2r_fixture.rs'):
    path = ROOT / name
    old = ROOT / '.threading-evidence/t2r/initial/source' / name
    command = ['rustfmt', '--check', '--edition', '2021', '--config', 'skip_children=true']
    if old.exists():
        spans = [{'file': str(path), 'range': [j+1, max(j+1, k)]}
                 for op, _, _, j, k in difflib.SequenceMatcher(
                     a=old.read_text().splitlines(), b=path.read_text().splitlines(),
                     autojunk=False).get_opcodes() if op != 'equal']
        command += ['--unstable-features', '--file-lines', json.dumps(spans)]
    subprocess.run(command + [str(path)], cwd=ROOT, check=True)
print('T2R changed spans and new fixture pass rustfmt checks.')
