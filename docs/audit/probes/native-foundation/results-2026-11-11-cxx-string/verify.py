#!/usr/bin/env python3
"""Verify Phase 21 native results, preserved inputs, faults and resource recovery."""
from pathlib import Path
import csv
import hashlib
import json
import re
import runpy
import subprocess
E=Path(__file__).resolve().parent
ROOT=E.parents[4]
runpy.run_path(str(E/'preservation_checks.py'))
original=json.loads((E/'historical.json').read_text())
for name in original:
    path=ROOT/name
    assert path.read_bytes()==subprocess.check_output(['git','show','HEAD:'+name],cwd=ROOT),name
for name in ('entry','write','exit'):
    name='docs/audit/probes/native-foundation/'+name+'.c'
    assert (ROOT/name).read_bytes()==subprocess.check_output(['git','show','HEAD:'+name],cwd=ROOT)
for name in ('tools/hayro-native-probe/Cargo.lock','tools/native-std-probe/Cargo.lock','tools/tests/fd-description/Cargo.lock'):
    assert (ROOT/name).read_bytes()==subprocess.check_output(['git','show','HEAD:'+name],cwd=ROOT)
summary={'preservation':'PASS','historical_unchanged':True,'cpu':{}}
for cpu in (2,4):
    d=E/f'accepted-{cpu}cpu'
    rows=list(csv.DictReader((d/'results.csv').open()))
    built=[x for x in rows if x['build']=='PASS']
    assert len(built)==80 and all(x['acceptance']=='PASS' and x['guest_run']=='PASS' for x in built)
    history=[x for x in rows if x['historical']=='True']
    assert len(history)==21 and sum(x['acceptance']=='PASS' for x in history)==20
    failures=[x for x in history if x['acceptance']!='PASS']
    assert len(failures)==1 and failures[0]['probe']=='cpp-runtime' and failures[0]['build']=='FAIL' and failures[0]['guest_run']=='NOT_RUN'
    assert "'stdexcept' file not found" in (d/'build-cpp-runtime.log').read_text()
    assert (d/'faults.log').read_bytes()==b''
    serial=(d/'serial.log').read_text()
    reaps=re.findall(r'\[NATIVE-C-REAP-END\] case=(\d+) exit=-?\d+ user_frames=(\d+) page_tables=(\d+) owned_frames=(\d+) fds=(\d+) stack=(\d+)',serial)
    assert len(reaps)==103 and all(x[3:]==('0','0','0') for x in reaps)
    primary=[x for x in reaps if int(x[1])>0]
    assert len(primary)==80 and {int(x[0]) for x in primary}==set(range(80))
    clean=re.findall(r'\[NATIVE-C-RESULT\] (.*)',serial)
    assert len(clean)==80
    for line in clean:
        fields=dict(re.findall(r'(\w+)=(-?\d+)',line))
        assert fields['clean']=='1'
        for prefix in ('file','private','pt','handles'):
            assert fields[prefix+'_before']==fields[prefix+'_after'],line
    refs=re.findall(r'\[NATIVE-C-INET\] before=(\d+) after=(\d+)',serial)
    assert len(refs)==80 and all(a==b for a,b in refs)
    assert '[NATIVE-C-DONE] cases=80' in serial
    results=json.loads((d/'results.json').read_text())
    assert results['cxx-string']['stdout']=='string:A\0B:42\n'
    assert results['cxx-string-backend']['stdout']=='A\0BCDzeer'
    for mode in ('allocation','overflow','null'):
        r=results['cxx-string-'+mode]
        assert r['exit']==134 and r['stdout']=='armed\n' and r['stderr']=='' and r['pass_']
    assert results['cxx-backend']['stdout']=='partial\nabxy'
    assert results['cxx-output']['stderr']=='stderr\nclog\n'
    assert results['epoll-lifecycle']['pass_']
    fp=(E/f'native-std-fp-{cpu}cpu.log').read_text()
    assert '[NATIVE-STD] x87/XMM/MXCSR preemption isolation OK' in fp
    assert '[NATIVE-STD] native PDF parser and vector raster OK' in fp
    assert not re.search(r'KERNEL PANIC|\[EXCEPTION\]|FP isolation FAILED|parser/raster FAILED',fp)
    summary['cpu'][cpu]={'native_pass':80,'historical_pass':20,'historical_build_fail':1,'cpp_runtime':'NOT_RUN','clean_primary_reaps':80,'total_reaps':103,'faults':0,'fp':'PASS'}
assert '121 passed; 0 failed' in (E/'run_unistd/libc.log').read_text()
assert '9 passed; 0 failed' in (E/'alloc-base.log').read_text()
gates=json.loads((E/'regressions.json').read_text())
assert gates['workspace']==101 and gates['run_stdio']==1 and gates['alloc']==1
assert all(code==0 for name,code in gates.items() if name not in ('workspace','run_stdio','alloc'))
assert 'inet_socket' in (E/'run_stdio/host-fd-vfs.log').read_text()
assert 'GLOBAL_ALLOC' in (E/'alloc-driver.log').read_text()
assert (E/'string-abi-target/archive-members.log').read_text().count('string.o')==1
summary['separate_failures']={'workspace':'FAIL','legacy_stdio':'BLOCKED: missing host harness module','allocator_global_proof':'BUILD_FAIL: host cfg excludes GLOBAL_ALLOC'}
(E/'acceptance-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
