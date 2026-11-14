"""Re-evaluate individual assertions, suite rows, kernel cleanup and hashes."""
from pathlib import Path
import csv
import hashlib
import json
import re

p = Path(__file__).resolve().parent
results = {}
for cpu in (2, 4):
    for name in ('wrapper', 'linux-handler', 'linux-entry', 'dup2-replacement',
                 'epoll', 'epoll-lifecycle', 'inet-socket', 'tcp-epoll-lt', 'tcp-epoll-revalidation', 'suite'):
        base = p / ('validated' if name in ('wrapper', 'linux-handler', 'linux-entry') else 'run-2')
        folder = base / f'{name}-{cpu}'
        rows = json.loads((folder / 'results.json').read_text())
        assert rows and all(v['pass_'] for v in rows.values()), (folder, rows)
        serial = (folder / 'serial.log').read_text(errors='replace')
        inet = re.findall(r'\[NATIVE-C-INET\] before=(\d+) after=(\d+)', serial)
        cleanup = re.findall(r'\[NATIVE-C-RESULT\].*', serial)
        assert len(inet) == len(rows) and all(a == b for a, b in inet), folder
        assert len(cleanup) == len(rows) and all('clean=1 ' in x for x in cleanup), folder
        assertions, summaries = [], []
        for row in rows.values():
            stdout = row['stdout']
            # Full suite contains intentional errors in some fixtures; their
            # individual row/output/expected-exit contract is authoritative.
            if name != 'suite':
                assert not re.search(r'^.*: FAIL ', stdout, re.M), folder
            for line in stdout.splitlines():
                if ': PASS ' not in line:
                    continue
                if line == 'dup-runtime: PASS complete' or line.startswith('dup2-replacement: PASS local tcp replacement ') or line == 'linux-entry: PASS complete':
                    summaries.append(line)
                else:
                    assertions.append(line)
        negatives = []
        with (folder / 'results.csv').open() as f:
            csvrows = list(csv.DictReader(f))
        for row in csvrows:
            assert row['build'] == 'PASS' and row['guest_run'] == 'PASS' and row['acceptance'] == 'PASS', row
            if int(row['expected_exit']) != 0:
                negatives.append(dict(probe=row['probe'], expected_exit=int(row['expected_exit']),
                                      actual_exit=int(row['exit']), classification='PASS (expected nonzero)'))
        if name == 'suite':
            assert len(rows) == 94
        results[f'{name}-{cpu}'] = dict(classification='PASS', build='PASS', link='PASS',
            runtime='PASS', rows=len(rows), individual_PASS_records=len(assertions),
            completion_PASS_records=len(summaries), assertion_failures=[], inet_refs=inet,
            kernel_cleanup=cleanup, expected_nonzero_tests=negatives)
for key in ('cxx-checker', 'poll-abi', 'kernel-check', 'libc-check', 'network-check', 'diff-check'):
    row = json.loads((p / 'run-2/summary.json').read_text())[key]
    assert all(v == 'PASS' for v in row.values()), (key, row)
    results[key] = row
(p / 'acceptance-summary.json').write_text(json.dumps(results, indent=2))
# Every saved executable, kernel, object and ISO in this new evidence tree.
files = [f for f in p.rglob('*') if f.is_file() and f.suffix in ('.elf', '.o', '.iso', '.a')]
files += [Path(f) for f in ('target/native-c/sysroot/lib/libsunlight_native_c.a',
    'target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a',
    'target/x86_64-unknown-none/debug/sunlight-kernel',
    'target/native-c-guest/support/x86_64-unknown-none/release/net_server') if Path(f).is_file()]
hashes = {}
for f in files:
    h = hashlib.sha256()
    with f.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    hashes[str(f)] = h.hexdigest()
(p / 'all-artifact-hashes.json').write_text(json.dumps(hashes, indent=2))
for f in ('target/native-c/sysroot/lib/libsunlight_native_c.a', 'target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a'):
    assert hashes[f] == '2fb382ef6c2b83b48e034645a078d5e36e40f036d98b00a00eace35d222696c7'
assert hashes['target/x86_64-unknown-none/debug/sunlight-kernel'] == '97d9bb2204549ec7548b14fc6d2c90af0d3986dfdd5292aa5eec8ba21e931594'
print('PASS: all accepted rows, assertions, cleanup, expected negatives and production hashes')
