"""Read-only Linux QEMU thread observations, in the host monotonic clock domain.

These samples bound host runnable wait and runtime; they are not guest IRQ
delivery timestamps. Serial offsets allow trial-level association only.
"""
import json
from pathlib import Path
import threading
import time


class HostObserver:
    def __init__(self, pid, serial, output, interval=0.01):
        self.pid, self.serial, self.output = pid, serial, output
        self.interval = interval
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.collect, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join()

    def collect(self):
        with self.output.open('w') as stream:
            while not self.stop.is_set():
                begin = time.monotonic_ns()
                tasks = []
                try:
                    for task in sorted(Path(f'/proc/{self.pid}/task').iterdir()):
                        try:
                            stat = [int(n) for n in (task / 'schedstat').read_text().split()]
                            tasks.append(dict(tid=int(task.name), name=(task / 'comm').read_text().strip(),
                                              runtime_ns=stat[0], runqueue_wait_ns=stat[1], timeslices=stat[2]))
                        except (OSError, ValueError, IndexError):
                            continue
                    offset = self.serial.stat().st_size
                except OSError:
                    break
                stream.write(json.dumps(dict(begin_ns=begin, end_ns=time.monotonic_ns(),
                                             serial_bytes=offset, tasks=tasks)) + '\n')
                self.stop.wait(self.interval)
