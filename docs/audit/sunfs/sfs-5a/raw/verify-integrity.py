#!/usr/bin/env python3
"""Verify/seal only SFS-5A outputs. Does not rewrite prior milestone evidence."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[5]
OUT = ROOT/'docs/audit/sunfs/sfs-5a'


def sha(p):
    h = hashlib.sha256()
    with p.open('rb') as f:
        for b in iter(lambda: f.read(1024*1024), b''):
            h.update(b)
    return h.hexdigest()


baseline = json.loads((OUT/'source-baseline.json').read_text())
changed = [n for n, h in baseline['sha256'].items() if not (ROOT/n).is_file() or sha(ROOT/n) != h]
allowed = {'kernel/Cargo.toml', 'kernel/src/main.rs', 'sunlight-virtio/src/blk.rs'}
names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT).decode().split('\0')
new = [n for n in names if n and n not in baseline['sha256'] and (ROOT/n).is_file() and not (ROOT/n).is_symlink()]
allowed_new = {'kernel/src/sfs5a_test.rs', 'tools/sunfs5a_validate.py', 'docs/filesystems/sunfs/sfs-5a-native-block-durability.md'}
unexpected = sorted(set(changed)-allowed)
unexpected_new = [n for n in new if n not in allowed_new and not n.startswith('docs/audit/sunfs/sfs-5a/')]
diff = subprocess.run(['git', 'diff', '--check'], cwd=ROOT, capture_output=True, text=True)
assert not (unexpected or unexpected_new or diff.returncode), (unexpected, unexpected_new, diff.stdout)
prior = [n for n in changed if n.startswith('docs/audit/sunfs/')]
assert not prior, prior
outputs = {n:sha(ROOT/n) for n in sorted(set(changed+new)) if n != 'docs/audit/sunfs/sfs-5a/source-integrity.json'}
result = dict(status='PASS', incoming_revision=baseline['revision'], baseline_status=baseline['status'],
              unexpected_modified=unexpected, unexpected_new=unexpected_new, prior_evidence_preserved=True,
              incoming_regular_files=len(baseline['sha256']), modified_incoming=changed,
              new_files=new, git_diff_check=diff.returncode, sha256=outputs,
              scope='tracked and unignored regular files; ignored build outputs and symlink metadata excluded')
if '--seal' in sys.argv:
    (OUT/'source-integrity.json').write_text(json.dumps(result, indent=2)+'\n')
else:
    old = json.loads((OUT/'source-integrity.json').read_text())
    assert old['sha256'] == outputs, 'sealed outputs changed'
print('SFS5A integrity PASS; all prior SunFS evidence unchanged')
