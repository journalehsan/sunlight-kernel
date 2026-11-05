#!/usr/bin/env python3
"""Check T3 Rust edits without attributing old dirty-tree formatting to T3."""
import difflib,json,subprocess,tempfile
from pathlib import Path
root=Path(__file__).resolve().parents[1]
initial=root/'.threading-evidence/t3/initial/source'
for old in sorted(initial.rglob('*.rs')):
    path=root/old.relative_to(initial)
    spans=[{'file':str(path),'range':[j+1,max(j+1,k)]}
           for op,_,_,j,k in difflib.SequenceMatcher(a=old.read_text().splitlines(),b=path.read_text().splitlines(),autojunk=False).get_opcodes() if op!='equal']
    changed = {line for span in spans for line in range(span['range'][0],span['range'][1]+1)}
    with tempfile.TemporaryDirectory() as tmp:
        candidate=Path(tmp)/path.name
        candidate.write_text(path.read_text())
        subprocess.run(['rustfmt','--edition','2021','--config','skip_children=true',str(candidate)],check=True)
        for op,i,j,_,_ in difflib.SequenceMatcher(a=path.read_text().splitlines(),b=candidate.read_text().splitlines(),autojunk=False).get_opcodes():
            if op!='equal' and set(range(i+1,max(i+1,j)+1)) & changed:
                raise RuntimeError(f'T3 formatting discrepancy: {path}:{i+1}')
for name in ('cpu_identity','cpu_topology'):
    subprocess.run(['rustfmt','--check','--edition','2021',str(root/f'kernel/src/arch/x86_64/{name}.rs')],check=True)
print('T3 changed Rust spans and new modules: formatted')
