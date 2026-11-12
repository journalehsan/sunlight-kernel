from pathlib import Path
import csv, json, subprocess

out = Path(__file__).resolve().parent
root = Path.cwd()
expected_history = b'exception OK\nTLS OK\nstatic initialization OK\natexit OK\n'
expected_lifecycle = (b'global-init\nworker:TLS\nTLS-done\nmain:global:TLS\n'
                      b'TLS-done\natexit\nglobal-done\n')
local_static = {
    'basic': (b'basic:OK\nbasic:D\n', 0),
    'concurrent': (b'concurrent:OK\nconcurrent:D\n', 0),
    'retry': (b'retry:OK\nretry:D\n', 0),
    'nested': (b'nested:OK\nouter:D\ninner:D\n', 0),
    'order': (b'order:OK\nsecond:D\nfirst:D\n', 0),
    'recursive': (b'recursive:armed\n', 134),
}
summary = {}
for cpus in (2, 4):
    base = out / f'{cpus}cpu'
    rows = list(csv.DictReader((base / 'results.csv').open()))
    built = [r for r in rows if r['build'] == 'PASS']
    assert len(built) == 93 and all(r['acceptance'] == 'PASS' for r in built)
    original = next(r for r in rows if r['probe'] == 'cpp-runtime')
    assert original['acceptance'] == 'LINK_FAIL' and original['guest_run'] == 'NOT_RUN'
    assert (base / 'cpp-runtime-native.stdout.txt').read_bytes() == expected_history
    assert (base / 'cpp-runtime-native.stderr.txt').read_bytes() == b''
    assert not (base / 'faults.log').read_bytes()
    for mode, (stdout, code) in local_static.items():
        row = next(r for r in rows if r['probe'] == 'cxx-local-static-' + mode)
        assert row['acceptance'] == 'PASS' and row['exit'] == str(code)
        assert (base / ('cxx-local-static-' + mode + '.stdout.txt')).read_bytes() == stdout
    assert b'__cxa_guard_acquire detected recursive initialization' in (
        base / 'cxx-local-static-recursive.stderr.txt').read_bytes()
    life = out / f'lifecycle-{cpus}cpu'
    lrows = list(csv.DictReader((life / 'results.csv').open()))
    assert len(lrows) == 1 and lrows[0]['acceptance'] == 'PASS' and lrows[0]['exit'] == '0'
    assert (life / 'elf.stdout.txt').read_bytes() == expected_lifecycle
    assert not (life / 'faults.log').read_bytes()
    hist = out / f'historical-native-{cpus}cpu'
    hrows = list(csv.DictReader((hist / 'results.csv').open()))
    assert len(hrows) == 1 and hrows[0]['acceptance'] == 'PASS' and hrows[0]['exit'] == '0'
    assert (hist / 'elf.stdout.txt').read_bytes() == expected_history
    assert not (hist / 'faults.log').read_bytes()
    summary[str(cpus)] = {
        'ram_mib': 2048,
        'built_native_cases': 93,
        'native_pass': 93,
        'local_static_guards': 'PASS',
        'configured_historical': 'PASS',
        'original_command': 'LINK_FAIL; runtime NOT_RUN',
        'lifecycle': 'PASS exact output',
        'faults': 'none',
    }
original_log = (out / 'historical-original.log').read_text()
assert 'undefined symbol' in original_log and 'linker command failed' in original_log
for symbol in ('std::cout', '__cxa_allocate_exception', '_Unwind_Resume', 'pthread_create'):
    assert symbol in original_log, symbol
probe_paths = [p for p in subprocess.check_output(
    ['git', 'ls-files', 'docs/audit/probes'], text=True).splitlines()
    if p.endswith(('.c', '.cpp', '.py'))]
for f in probe_paths:
    assert (root / f).read_bytes() == subprocess.check_output(['git', 'show', 'HEAD:' + f], cwd=root), f
summary['historical_integrity'] = {'files': len(probe_paths), 'result': 'PASS'}
(out / 'acceptance-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
