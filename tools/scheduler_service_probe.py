#!/usr/bin/env python3
"""Exercise production RR dispatch and service accounting with a host clock.

Architecture/context and reclamation operations are stubbed; the real dispatcher,
queue publication, priority decision and runtime/slice bookkeeping are extracted.
Physical handoff/reclamation is covered separately by the Phase B guest gate.
"""
from pathlib import Path
import re
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
source = (root / 'kernel/src/sched/mod.rs').read_text()


def function(name, source=source):
    start = source.index('fn ' + name + '(')
    end = source.index('{', start) + 1
    depth = 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


fixture = (root / 'tools/tests/scheduler_service.rs').read_text()
methods = ['schedule_tick', 'priority_preemption_candidate', 'retain_priority_request', 'start_charging_runtime',
           'stop_execution_clock', 'account_current_runtime', 'account_and_apply_churn_penalty',
           'queue_process_to_core', 'enqueue_process_to_core', 'set_state',
           'pick_next_round_robin']
helpers = ['quantum_ticks', 'calculate_quantum_with_nice', 'on_task_ran',
           'update_burst_score', 'accumulate_counter']
constants = '\n'.join(re.findall(
    r'(?:pub )?const (?:BURST_\w+|MINIMUM_AGED_BURST_SCORE|AGING_THRESHOLD_TICKS|'
    r'MAX_CREDIT|MAX_DEBT|PROMOTE_LIMIT|SKIP_LIMIT|DECAY_RATE|STARVATION_BOOST|'
    r'QUANTUM_MIN|QUANTUM_MAX|SHORT_BURST_NS|SHORT_BURST_PENALTY):[^;]+;', source))
code = fixture.replace('// PRODUCTION_METHODS', '\n'.join(function(n) for n in methods))
code += '\n' + constants + '\n' + '\n'.join(function(n) for n in helpers)
code += '\n' + function('initial_embedded_nice', (root / 'kernel/src/process/spawn.rs').read_text())
for name, path in [('quantum', 'kernel/src/sched/quantum.rs'),
                   ('cpu_accounting', 'ipc/src/cpu_accounting.rs')]:
    code += f'\n#[path = "{root / path}"] mod {name};\n'
with tempfile.TemporaryDirectory(prefix='scheduler-service-') as temporary:
    path = Path(temporary)
    (path / 'probe.rs').write_text(code)
    subprocess.run(['rustc', '--edition=2021', '--test', str(path / 'probe.rs'),
                    '-o', str(path / 'probe')], check=True)
    subprocess.run([str(path / 'probe'), '--test-threads=1'], check=True, timeout=60)
