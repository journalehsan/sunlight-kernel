#!/usr/bin/env python3
"""Read ABI-v4 CPU telemetry from an isolated QEMU VM without guest agents.

Example: python3 tools/cpu_accounting_capture.py --socket /tmp/cpu-audit-qmp.sock \
  --kernel target/x86_64-unknown-none/debug/sunlight-kernel --seconds 60 \
  --label desktop-idle --output target/desktop-idle.jsonl
Requires a kernel built with cpu_accounting_diag for wake/scheduler counters.
No stop/resume: sequence validation rejects concurrent telemetry publications.
"""
import argparse
import json
import socket
import struct
import subprocess
import tempfile
import time
from pathlib import Path


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--socket', required=True)
    p.add_argument('--kernel', required=True)
    p.add_argument('--seconds', type=float, default=60)
    p.add_argument('--period', type=float, default=5)
    p.add_argument('--label', required=True)
    p.add_argument('--output', required=True)
    a = p.parse_args()
    if a.seconds <= 0 or a.period <= 0:
        p.error('seconds and period must be positive')
    symbols = subprocess.check_output(['nm', a.kernel], text=True)
    symbol = next(line for line in symbols.splitlines() if line.endswith('telemetry9TELEMETRY'))
    address = int(symbol.split()[0], 16)
    sock = socket.socket(socket.AF_UNIX)
    sock.connect(a.socket)
    stream = sock.makefile('rwb', buffering=0)
    stream.readline()

    def command(name, arguments=None):
        stream.write((json.dumps({'execute': name, 'arguments': arguments or {}})+'\n').encode())
        while True:
            result = json.loads(stream.readline())
            if 'error' in result:
                raise RuntimeError(result['error'])
            if 'return' in result:
                return result['return']

    command('qmp_capabilities')
    with tempfile.TemporaryDirectory(prefix='sunlight-cpu-') as temporary, open(a.output, 'w') as output:
        page_path = Path(temporary)/'page.bin'
        seq_path = Path(temporary)/'seq.bin'

        def read_sample():
            for _ in range(3):
                for addr, size, path in [(address, 12128, page_path), (address+12, 4, seq_path)]:
                    reply = command('human-monitor-command', {'command-line': f'memsave {addr:#x} {size} "{path}"'})
                    if reply:
                        raise RuntimeError(reply)
                data = page_path.read_bytes()
                magic, version, sequence = struct.unpack_from('<QII', data)
                if magic != 0x53554E4C54494D45 or version != 4:
                    raise RuntimeError('Expected telemetry ABI v4; check kernel/VM image pairing')
                if sequence & 1 or sequence != struct.unpack('<I', seq_path.read_bytes())[0]:
                    continue
                now = struct.unpack_from('<Q', data, 80)[0]
                count = min(struct.unpack_from('<I', data, 3932)[0], 64)
                cores = [struct.unpack_from('<10Q', data, 6752+80*i) for i in range(count)]
                tasks = {}
                for i in range(min(struct.unpack_from('<I', data, 88)[0], 64)):
                    pid, _, state, name, runtime, _, generation = struct.unpack_from('<IIB3x32sQII', data, 92+60*i)
                    tasks[(pid, generation)] = (name.rstrip(b'\0').decode('utf-8', 'replace'), runtime, state)
                return now, cores, tasks
            return None

        previous = read_sample()
        end = time.monotonic()+a.seconds
        while time.monotonic() < end:
            time.sleep(min(a.period, max(0, end-time.monotonic())))
            current = read_sample()
            if current is None or previous is None:
                previous = current
                continue
            now, cores, tasks = current
            old_now, old_cores, old_tasks = previous
            if now <= old_now or len(cores) != len(old_cores):
                continue
            dt = now-old_now
            delta = [[max(0, x-y) for x, y in zip(c, old)] for c, old in zip(cores, old_cores)]
            capacity = sum(c[0] for c in delta)
            busy = sum(c[1] for c in delta)
            idle = sum(c[2] for c in delta)
            if not capacity:
                continue
            top = []
            for identity, (name, runtime, state) in tasks.items():
                if identity in old_tasks:
                    runtime_delta = max(0, runtime-old_tasks[identity][1])
                    top.append({'pid': identity[0], 'name': name, 'state': state,
                                'runtime_ns': runtime_delta, 'cpu_percent': runtime_delta/(dt*len(cores))*100})
            record = {'label': a.label, 'sample_ns': now, 'interval_ns': dt,
                      'capacity_ns': capacity, 'busy_ns': busy, 'idle_ns': idle,
                      'cpu_percent': busy/capacity*100, 'idle_percent': idle/capacity*100,
                      'cores_percent': [c[1]/c[0]*100 if c[0] else None for c in delta],
                      'switches_s': sum(c[3] for c in delta)*1e9/dt,
                      'schedules_s': sum(c[4] for c in delta)*1e9/dt,
                      'wakeups_s': sum(c[5] for c in delta)*1e9/dt,
                      'timer_irqs_s': sum(c[8] for c in delta)*1e9/dt,
                      'ipc_wakeups_s': sum(c[9] for c in delta)*1e9/dt,
                      'top_tasks': sorted(top, key=lambda t: t['runtime_ns'], reverse=True)[:8]}
            output.write(json.dumps(record)+'\n')
            output.flush()
            print(json.dumps({k: v for k, v in record.items() if k != 'top_tasks'}), flush=True)
            previous = current
    sock.close()


if __name__ == '__main__':
    main()
