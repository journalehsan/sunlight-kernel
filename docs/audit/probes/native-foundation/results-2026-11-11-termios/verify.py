"""Verify recorded final guest and preservation evidence without rerunning guests."""
import csv
import hashlib
import json
from pathlib import Path
import re
import subprocess

root = Path(__file__).resolve().parents[5]
# Locate from the evidence path rather than relying on invocation cwd.
assert (root / 'Cargo.toml').exists()
out = Path(__file__).resolve().parent
summary = {}
for cpu in (2, 4):
    p = out / f'acceptance-{cpu}cpu'
    rows = list(csv.DictReader((p / 'results.csv').open()))
    executable = [r for r in rows if r['build'] == 'PASS']
    assert len(executable) == 71
    assert all(r['acceptance'] == 'PASS' and r['guest_run'] == 'PASS' for r in executable)
    assert {r['probe'] for r in rows if r['build'] != 'PASS'} == {'wchar', 'cpp-runtime'}
    assert not (p / 'faults.log').read_bytes()
    serial = (p / 'serial.log').read_text()
    reaps = re.findall(r'^\[NATIVE-C-REAP-END\].*$', serial, re.M)
    # Pthread children also emit reap markers; match every primary case/exit,
    # and require clean resource state on all markers, including children.
    assert len(reaps) >= 71
    assert all(line.endswith('owned_frames=0 fds=0 stack=0') for line in reaps)
    for number, row in enumerate(executable):
        assert re.search(rf'^\[NATIVE-C-REAP-END\] case={number} exit={row["exit"]} .*owned_frames=0 fds=0 stack=0$', serial, re.M)
    assert len(re.findall(r'^\[NATIVE-C-RESULT\].*clean=1 ', serial, re.M)) == 71
    assert len(re.findall(r'^\[NATIVE-C-INET\] before=0 after=0$', serial, re.M)) == 71
    for name in ('termios', 'termios-contract', 'termios-terminal-identity', 'epoll-lifecycle'):
        assert not (p / (name + '.stdout.txt')).read_bytes()
        assert not (p / (name + '.stderr.txt')).read_bytes()
    summary[str(cpu)] = dict(executable_pass=71, historical_pass=19, historical_build_fail=2,
                            fault_bytes=0, clean_reap=71)
for name in ('termios', 'termios-contract', 'termios-terminal-identity'):
    assert (out / 'acceptance-2cpu' / (name+'.elf')).read_bytes() == (out / 'acceptance-4cpu' / (name+'.elf')).read_bytes()
entry = json.loads((out / 'preservation.json').read_text())
for filename, digest in (entry['historical'] | entry['foundation_reference']).items():
    data = (root / filename).read_bytes()
    assert hashlib.sha256(data).hexdigest() == digest, filename
    assert data == subprocess.check_output(['git', 'show', 'HEAD:'+filename], cwd=root), filename
summary['historical_unchanged'] = len(entry['historical'] | entry['foundation_reference'])
assert all(code == 0 for name,code in json.loads((out/'regressions.json').read_text()).items() if name != 'workspace')
assert json.loads((out/'regressions.json').read_text())['workspace'] == 101
assert '120 passed; 0 failed' in (out/'host/libc.log').read_text()
for cpu in (2,4):
    text=(out/f'native-std-fp-{cpu}cpu.log').read_text()
    for marker in ('x87/XMM/MXCSR preemption isolation OK', 'native PDF parser and vector raster OK'):
        assert marker in text
    for marker in ('[EXCEPTION]', 'KERNEL PANIC', 'PAGE FAULT', 'FP isolation FAILED'):
        assert marker not in text
assert (out/'foundation-driver.log').read_text().rstrip().endswith('exit=0')
(out/'acceptance-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
