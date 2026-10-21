#!/usr/bin/env python3
"""Lint the actual kernel compiler invocation against a read-only Git archive.

Cargo's strict build stops in existing dependency/build-script diagnostics.
This supplemental pass uses the already built dependencies and exact rustc
arguments to reach the primary crate. No lint is suppressed in either pass.
"""
import io
import json
import os
from pathlib import Path
import shlex
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/fastlane-f13na-evidence/primary-lint'


def diagnostics(raw):
    found = {}
    for line in raw.splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        code = (item.get('code') or {}).get('code', '')
        if not code.startswith('clippy::') or item.get('level') not in ('error', 'warning'):
            continue
        for span in item.get('spans', []):
            if span.get('is_primary'):
                key = json.dumps([code, span['file_name'], [row['text'] for row in span['text']]])
                found[key] = dict(code=code, file=span['file_name'], line=span['line_start'],
                                  message=item['message'])
    return found


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    command = ['cargo', 'rustc', '-vv', '-p', 'sunlight-kernel', '--bin', 'sunlight-kernel',
               '--features', 'native_doorbell_test', '--offline', '--', '-C',
               f'metadata=f13na_primary_lint_{time.time_ns()}']
    with (OUT / 'compiler-invocation.log').open('w') as log:
        subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    raw = (OUT / 'compiler-invocation.log').read_text()
    commands = [line.split('Running `', 1)[1].rsplit('`', 1)[0] for line in raw.splitlines()
                if 'Running `' in line and '--crate-name sunlight_kernel ' in line]
    if len(commands) != 1:
        raise RuntimeError('one actual primary-crate rustc invocation required')
    words = shlex.split(commands[0])
    env = dict(os.environ)
    while words and '=' in words[0] and not words[0].startswith('/'):
        key, value = words.pop(0).split('=', 1)
        env[key] = value
    words[0] = subprocess.check_output(['rustup', 'which', 'clippy-driver'], text=True).strip()
    for index, word in enumerate(words):
        if word.startswith('--emit='):
            words[index] = '--emit=metadata'
    if '--json=diagnostic-rendered-ansi,artifacts,future-incompat' in words:
        words.remove('--json=diagnostic-rendered-ansi,artifacts,future-incompat')
    words += ['-D', 'warnings']
    env['CLIPPY_ARGS'] = ''
    archive = subprocess.check_output(['git', 'archive', 'HEAD', 'kernel'], cwd=ROOT)
    baseline = OUT / 'baseline'
    baseline.mkdir(exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
        bundle.extractall(baseline, filter='data')
    for path in ROOT.iterdir():
        destination = baseline / path.name
        if path.name not in ('kernel', '.git') and not destination.exists():
            destination.symlink_to(path, target_is_directory=path.is_dir())
    results = {}
    for label, cwd in [('current', ROOT), ('baseline', baseline)]:
        output = OUT / label
        output.mkdir(exist_ok=True)
        args = words.copy()
        args[args.index('--out-dir') + 1] = str(output)
        log = OUT / f'{label}.jsonl'
        with log.open('w') as handle:
            code = subprocess.run(args, cwd=cwd, env=env, stdout=handle, stderr=subprocess.STDOUT).returncode
        results[label] = dict(exit_code=code, diagnostics=diagnostics(log.read_text(errors='replace')))
    current, previous = results['current']['diagnostics'], results['baseline']['diagnostics']
    results['current_only'] = [value for key, value in current.items() if key not in previous]
    results['baseline_only'] = [value for key, value in previous.items() if key not in current]
    results['command'] = command
    results['source_revision'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    (OUT / 'comparison.json').write_text(json.dumps(results, indent=2) + '\n')
    print('Primary Clippy: baseline=%d current=%d current-only=%d' %
          (len(previous), len(current), len(results['current_only'])), flush=True)
    if results['current_only'] or not previous:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
