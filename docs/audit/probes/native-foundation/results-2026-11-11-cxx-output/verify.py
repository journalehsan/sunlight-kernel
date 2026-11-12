#!/usr/bin/env python3
"""Independently check preserved sources and recorded native runtime evidence."""
from pathlib import Path
import csv
import hashlib
import json
import re
import subprocess
E = Path(__file__).resolve().parent
ROOT = E.parents[4]
def sha(data): return hashlib.sha256(data).hexdigest()
p = json.loads((E/'preservation.json').read_text())
for name, digest in p['incoming'].items():
    data = (ROOT/name).read_bytes()
    if name == 'docs/audit/std-progress.md':
        data = data[:p['audit_prefix_bytes']]
    if name == 'tools/native-c/guest.py':
        s = data.decode().replace("        if command('build-cxx-output-runtime', ['bash', 'tools/build_native_cxx.sh']):\n            return 1\n", '')
        a = s.index("        sources += [('cxx-output',")
        b = s.index("        sources += [('wchar-contract',", a)
        s = s[:a] + s[b:]
        a = s.index("            if not historical and src.suffix == '.cpp':")
        b = s.index("            if command('build-' + name", a)
        data = (s[:a] + s[b:]).encode()
    assert sha(data) == digest, 'incoming changed: ' + name
for name, digest in p['historical'].items():
    data = (ROOT/name).read_bytes()
    assert sha(data) == digest, name
    assert data == subprocess.check_output(['git', 'show', 'HEAD:'+name], cwd=ROOT), name
for name in ('entry', 'write', 'exit'):
    path = 'docs/audit/probes/native-foundation/'+name+'.c'
    assert (ROOT/path).read_bytes() == subprocess.check_output(['git','show','HEAD:'+path],cwd=ROOT)
for path in ('tools/hayro-native-probe/Cargo.lock', 'tools/native-std-probe/Cargo.lock', 'tools/tests/fd-description/Cargo.lock'):
    assert (ROOT/path).read_bytes() == subprocess.check_output(['git','show','HEAD:'+path],cwd=ROOT)
summary = {'incoming_preserved': len(p['incoming']), 'historical_unchanged': True, 'cpu': {}}
for cpu in (2, 4):
    d = E/f'final-{cpu}cpu'
    rows = list(csv.DictReader((d/'results.csv').open()))
    built = [x for x in rows if x['build']=='PASS']
    assert len(built)==75 and all(x['acceptance']=='PASS' and x['guest_run']=='PASS' for x in built)
    history = [x for x in rows if x['historical']=='True']
    assert len(history)==21 and sum(x['acceptance']=='PASS' for x in history)==20
    blocked = [x for x in history if x['acceptance']!='PASS']
    assert len(blocked)==1 and blocked[0]['probe']=='cpp-runtime' and blocked[0]['guest_run']=='NOT_RUN'
    assert "'string' file not found" in (d/'build-cpp-runtime.log').read_text()
    assert (d/'faults.log').read_bytes()==b''
    serial = (d/'serial.log').read_text()
    reaps = re.findall(r'\[NATIVE-C-REAP-END\] case=(\d+) exit=-?\d+ user_frames=(\d+) page_tables=(\d+) owned_frames=(\d+) fds=(\d+) stack=(\d+)',serial)
    assert len(reaps)==98 and all(x[3:]==('0','0','0') for x in reaps)
    primary = [x for x in reaps if int(x[1])>0]
    assert len(primary)==75 and {int(x[0]) for x in primary}==set(range(75))
    clean = re.findall(r'\[NATIVE-C-RESULT\] case=\d+ exit=-?\d+ stdout=\d+ stderr=\d+ clean=(\d+)',serial)
    assert clean==['1']*75
    refs = re.findall(r'\[NATIVE-C-INET\] before=(\d+) after=(\d+)',serial)
    assert len(refs)==75 and all(a==b for a,b in refs)
    results=json.loads((d/'results.json').read_text())
    assert results['cxx-backend']['stdout']=='partial\nabxy' and results['cxx-backend']['stderr']==''
    assert results['cxx-output']['stderr']=='stderr\nclog\n'
    assert results['epoll-lifecycle']['pass_']
    b = E/f'boundaries-{cpu}cpu'
    expanded=json.loads((b/'results.json').read_text())['elf']
    assert expanded['pass_'] and expanded['exit']==0 and expanded['stderr']=='stderr\nclog\n'
    expected='output 0 1 -9223372036854775808 9223372036854775807 18446744073709551615 18446744073709551615\nbounds -32768 32767 65535 -2147483648 2147483647 4294967295 -9223372036854775808 9223372036854775807 0\nABC\n'
    assert expanded['stdout']==expected and (b/'faults.log').read_bytes()==b''
    fp=(E/f'native-std-fp-{cpu}cpu.log').read_text()
    assert '[NATIVE-STD] x87/XMM/MXCSR preemption isolation OK' in fp
    assert '[NATIVE-STD] native PDF parser and vector raster OK' in fp
    assert not re.search(r'KERNEL PANIC|\[EXCEPTION\]|FP isolation FAILED|parser/raster FAILED',fp)
    summary['cpu'][cpu]={'buildable_pass':75,'historical_pass':20,'historical_build_fail':1,'clean_primary_reaps':75,'faults':0,'output_boundaries':'PASS','fp':'PASS'}
assert '121 passed; 0 failed' in (E/'host/libc.log').read_text()
assert all(code==0 for name,code in json.loads((E/'gate-results.json').read_text()).items() if name!='workspace')
extra=json.loads((E/'extra-results.json').read_text())
assert extra['stdio']==1 and all(code==0 for name,code in extra.items() if name!='stdio')
(E/'acceptance-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
