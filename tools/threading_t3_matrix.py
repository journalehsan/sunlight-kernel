#!/usr/bin/env python3
"""Run T3 regression suites sequentially; retain failures and continue coverage."""
import subprocess
from pathlib import Path
root = Path(__file__).resolve().parents[1]
folder = root / '.threading-evidence/t3'
if any((folder / ('final-' + suite + '.log')).exists() for suite in ('t1','admission','auth','linux','fp','std','production')):
    raise SystemExit('Retained matrix logs already exist; use threading_t3_regressions.py for fresh timestamped suite attempts.')
for suite in ('t1', 'admission', 'auth', 'linux', 'fp', 'std', 'production'):
    with (folder / ('final-' + suite + '.log')).open('x') as log:
        result = subprocess.run(['python3', 'tools/threading_t3_regressions.py', suite], cwd=root, stdout=log, stderr=subprocess.STDOUT)
    print(suite, result.returncode, flush=True)
