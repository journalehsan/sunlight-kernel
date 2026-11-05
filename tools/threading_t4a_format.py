#!/usr/bin/env python3
"""Format/check only T4A spans against the preserved dirty starting tree."""
import argparse
import difflib
import json
from pathlib import Path
import subprocess

ROOT=Path(__file__).resolve().parents[1]
INITIAL=ROOT/'.threading-evidence/t4a/initial/source'
OLD=('kernel/build.rs','kernel/src/arch/x86_64/syscall.rs','kernel/src/main.rs',
     'kernel/src/memory/user.rs','kernel/src/process/fd_table.rs','kernel/src/process/mmap.rs','kernel/src/process/mod.rs','kernel/src/process/spawn.rs','sunlight-libc/src/std_runtime.rs',
     'kernel/src/sched/mod.rs','kernel/src/sched/handoff.rs','sunlight-docserv/src/main.rs')
NEW=('kernel/src/sched/t4a_fixture.rs','tools/helios-probes/atomic-futex-probe.rs')
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--write',action='store_true');a=p.parse_args()
for name in OLD+NEW:
    path=ROOT/name
    command=['rustfmt','--edition','2021','--config','skip_children=true']
    if not a.write:command+=['--check']
    if name in OLD:
        spans=[{'file':str(path),'range':[j+1,max(j+1,k)]} for op,_,_,j,k in difflib.SequenceMatcher(a=(INITIAL/name).read_text().splitlines(),b=path.read_text().splitlines(),autojunk=False).get_opcodes() if op!='equal']
        command+=['--unstable-features','--file-lines',json.dumps(spans)]
    subprocess.run(command+[str(path)],check=True,cwd=ROOT)
print('T4A changed spans/new fixtures: rustfmt PASS')
