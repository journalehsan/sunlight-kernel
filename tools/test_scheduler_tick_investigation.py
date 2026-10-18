import json
from pathlib import Path
import tempfile
import unittest

from scheduler_tick_investigation import host_reaps, samples


class TickInvestigationTests(unittest.TestCase):
    def test_complete_and_right_censored_wakes_retain_individual_samples(self):
        run = dict(services=[dict(pid=6)], events=[
            [11, 10, 0, 6, 15, 42, 0], [3, 11, 0, 6, 1, 15, 42],
            [9, 12, 0, 6, 42, 20, 5], [4, 13, 0, 4, 42, 7, 15],
            [9, 14, 0, 6, 4, 15, 1], [4, 15, 0, 42, 4, 32, 20],
            [13, 16, 0, 42, 243, 1, 1], [9, 17, 0, 6, 42, 20, 5],
            [4, 18, 0, 6, 42, 32, 15],
            [11, 20, 0, 6, 15, 42, 0], [3, 21, 0, 6, 1, 15, 42],
            [9, 25, 0, 6, 42, 20, 5],
        ])
        records, _, _, _ = samples(run)
        wakes = [r for r in records if r['kind'] == 'wake']
        self.assertEqual([r['status'] for r in wakes], ['complete', 'right_censored'])
        self.assertEqual([r['ns'] for r in wakes], [8, 5])
        self.assertEqual(wakes[0]['decisions'], 3)
        self.assertEqual(wakes[0]['irqs'], 0)
        self.assertEqual(wakes[1]['end_ns'], 25)

    def test_host_brackets_use_serial_bytes_including_crlf_and_utf8(self):
        prefix = 'unrelated → log\r\n'.encode()
        start = b'[SCHED] ipc_entries_reaped_for_process pid=42 cleared_msgs=0\r\n'
        end = b'[PMM-DIAG] pid=42 owned_frames=0\r\n'
        offsets = [0, len(prefix+start), len(prefix+start), len(prefix+start),
                   len(prefix+start+end), len(prefix+start+end)]
        rows = [dict(begin_ns=i*10, end_ns=i*10+1, serial_bytes=offset,
                     tasks=[dict(tid=9, name='CPU 0/TCG', runtime_ns=i*8,
                                 runqueue_wait_ns=i*2)]) for i, offset in enumerate(offsets)]
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'host.jsonl'
            path.write_text(''.join(json.dumps(r)+'\n' for r in rows))
            result = host_reaps(path, prefix+start+end)
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]['runtime_upper_ns'], 40)
        self.assertEqual(result[0]['runtime_lower_ns'], 8)
        self.assertEqual(result[0]['runqueue_wait_upper_ns'], 10)


if __name__ == '__main__':
    unittest.main()
