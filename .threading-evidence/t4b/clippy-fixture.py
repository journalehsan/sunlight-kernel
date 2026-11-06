"""Run the actual kernel Clippy invocation after a normal Cargo build.

Avoid re-entering the unrelated native-std SVG builder under clippy-driver.
Cargo's exact rustc argv/environment is retained; only the compiler and output
directory/emission are changed. No diagnostics are suppressed.
"""
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

folder = Path(sys.argv[1]).resolve()
folder.mkdir(parents=True, exist_ok=True)
build = ['cargo', 'rustc', '-vv', '--package', 'sunlight-kernel', '--bin',
         'sunlight-kernel', '--offline', '--features', 'futex_lifecycle_test,futex_lifecycle_diag', '--', '--cfg', 'threading_t3_clippy_capture']
with (folder / 'cargo.log').open('w') as log:
    code = subprocess.run(build, stdout=log, stderr=subprocess.STDOUT).returncode
if code:
    raise SystemExit(code)
lines = (folder / 'cargo.log').read_text().splitlines()
line = next(line for line in reversed(lines)
            if 'Running `' in line and '--crate-name sunlight_kernel ' in line)
argv = shlex.split(line.split('Running `', 1)[1].rsplit('`', 1)[0])
environment = os.environ.copy()
while argv and '=' in argv[0] and not argv[0].startswith('/'):
    name, value = argv.pop(0).split('=', 1)
    environment[name] = value
argv[0] = subprocess.check_output(['rustup', 'which', 'clippy-driver'], text=True).strip()
for i, arg in enumerate(argv):
    if arg.startswith('--emit='):
        argv[i] = '--emit=metadata'
    if arg == '--out-dir':
        argv[i + 1] = str(folder)
record = {'command': argv, 'environment': {k: v for k, v in environment.items()
                                        if k.startswith(('CARGO_', 'OUT_DIR'))}}
with (folder / 'clippy.jsonl').open('w') as log:
    result = subprocess.run(argv, env=environment, stdout=log, stderr=subprocess.STDOUT)
record['exit_code'] = result.returncode
(folder / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
print('Direct actual-kernel Clippy exit:', result.returncode)
raise SystemExit(result.returncode)
