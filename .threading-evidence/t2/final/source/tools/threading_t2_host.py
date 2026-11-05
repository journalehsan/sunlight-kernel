#!/usr/bin/env python3
"""Compile production admission methods with explicit hardware-only host models."""
from pathlib import Path
import subprocess
import tempfile
import argparse
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--bore',action='store_true')
args=parser.parse_args()
ROOT = Path(__file__).resolve().parents[1]
fixture = (ROOT / 'tools/tests/thread_admission.rs').read_text()
helpers = '\n'.join(f'#[path = "{ROOT / "kernel/src/process" / (name + ".rs")}"] pub mod {name};'
                    for name in ('thread_admission', 'env'))
fixture = fixture.replace('// PRODUCTION_HELPERS', helpers).replace(
    '// PRODUCTION_ADMISSION', (ROOT / 'kernel/src/sched/thread_admission.rs').read_text().replace('//!', '//'))
if args.bore:
    fixture=fixture.replace('const SCHEDULER_MODE: SchedulerMode = SchedulerMode::RoundRobin;', 'const SCHEDULER_MODE: SchedulerMode = SchedulerMode::Bore;')
source = (ROOT / 'kernel/src/sched/mod.rs').read_text()
def function(name):
    start = source.index('fn ' + name + '(')
    end = source.index('{', start) + 1
    depth = 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]
fixture += '\nimpl Scheduler {\n' + '\n'.join(function(name) for name in (
    'live_address_space_borrowers', 'current_address_space_has_borrowers')) + '\n}\n'
with tempfile.TemporaryDirectory(prefix='t2-host-') as temporary:
    folder = Path(temporary)
    (folder / 'model.rs').write_text(fixture)
    subprocess.run(['rustc', '--edition=2021', '--test', str(folder / 'model.rs'),
                    '-o', str(folder / 'model')], check=True)
    subprocess.run([str(folder / 'model'), '--test-threads=1'], check=True, timeout=60)
