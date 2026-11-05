#!/usr/bin/env python3
"""Validate C1.2F traces and export every completed/edge-censored timer sample.

All subtraction is within the validated guest CPU clock. Host schedstat stays
separate; neither an IRQ gap nor a guest checkpoint identifies host descheduling.
"""
import argparse
import csv
import json
from pathlib import Path
import bisect
import re

from scheduler_timer import analyze, frames, quantiles


def samples(run):
    timer = run['services'][0]['pid']
    rows = run['events']
    pending = {}
    result, gaps, checkpoints, tails = [], [], [], []
    previous_irq = None
    previous_checkpoint = None
    for index, event in enumerate(rows):
        kind, now, cpu, pid, a, b, c = event
        if kind == 1:
            if previous_irq is not None:
                gaps.append(dict(begin_ns=previous_irq, end_ns=a, ns=a-previous_irq))
            previous_irq = a
        if kind == 12:
            if previous_checkpoint is not None:
                old = previous_checkpoint
                # Consecutive checkpoints delimit actual code regions, including
                # nested reap checkpoints within the enclosing dispatch call.
                checkpoints.append(dict(begin_ns=old[1], end_ns=now, ns=now-old[1],
                                        from_stage=old[4], to_stage=a, from_pid=old[3], to_pid=pid))
            previous_checkpoint = event
        elif kind in (4, 9):
            previous_checkpoint = None
        if kind == 11 and pid == timer:
            if 'wake' in pending:
                raise ValueError('duplicate outstanding timer wake')
            pending['wake'] = dict(kind='wake', begin_ns=now, nice=a-20, current=b, index=index)
        if kind == 3 and pid == timer:
            pending.setdefault('queue', dict(kind='queue', begin_ns=now, nice=b-20, current=c, notify=a, index=index))
        if kind == 4 and pid == timer:
            for key in ('wake', 'queue'):
                if key in pending:
                    item = pending.pop(key)
                    window = rows[item.pop('index'):index+1]
                    decisions = [e for e in window if e[0] == 9]
                    item.update(end_ns=now, ns=now-item['begin_ns'], status='complete',
                                decisions=len(decisions), irqs=sum(e[0] == 1 for e in window),
                                first_decision_ns=decisions[0][1] if decisions else None,
                                selection_reason=b)
                    result.append(item)
                    if key == 'wake' and item['ns'] >= 300_000_000:
                        tails.append(dict(sample=item.copy(), events=window))
    for item in pending.values():
        item.pop('index')
        item.update(end_ns=rows[-1][1], ns=rows[-1][1]-item['begin_ns'], status='right_censored')
        result.append(item)
    return result, gaps, checkpoints, tails


def notifications(run):
    pending, result = None, []
    known_empty = False
    for kind, now, cpu, pid, a, b, c in run['events']:
        if kind == 2:
            if pending is None:
                pending = dict(begin_ns=now, latest_ns=now, first_tick=a, latest_tick=a, publications=0)
            pending.update(latest_ns=now, latest_tick=a, publications=pending['publications']+1)
        elif kind == 11:
            # Actual blocked receive -> Ready proves the preceding publication
            # began this notification's residence, even in the first window.
            known_empty = True
        elif kind == 6 and a == 0:
            if pending is not None:
                result.append(dict(**pending, end_ns=now, ns=now-pending['begin_ns'],
                                   status='complete' if known_empty else 'left_censored'))
            pending = None
            known_empty = True
    if pending is not None:
        now = run['events'][-1][1]
        result.append(dict(**pending, end_ns=now, ns=now-pending['begin_ns'],
                           status='right_censored' if known_empty else 'both_edges_censored'))
    return result


def host_summary(path):
    previous = {}
    runtime, wait, gaps = [], [], []
    last = None
    with path.open() as stream:
        for line in stream:
            row = json.loads(line)
            if last is not None:
                gaps.append(row['begin_ns'] - last)
            last = row['begin_ns']
            for task in row['tasks']:
                if '/TCG' not in task['name']:
                    continue
                old = previous.get(task['tid'])
                if old:
                    runtime.append(task['runtime_ns'] - old['runtime_ns'])
                    wait.append(task['runqueue_wait_ns'] - old['runqueue_wait_ns'])
                previous[task['tid']] = task
    return dict(sample_gap=quantiles(gaps), vcpu_runtime_delta=quantiles(runtime),
                vcpu_runqueue_wait_delta=quantiles(wait),
                total_vcpu_runtime_ns=sum(runtime), total_vcpu_runqueue_wait_ns=sum(wait))


def host_reaps(path, raw):
    """Conservative host-counter brackets around the serial lines surrounding
    the frame-owner scan. No guest/host clock subtraction or fitted clock map.
    Runqueue wait excludes sleeping and is not all possible host/TCG delay.
    """
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    offsets = [r['serial_bytes'] for r in rows]
    starts, result = {}, []
    def counters(row):
        ts = [t for t in row['tasks'] if '/TCG' in t['name']]
        return ts[0] if len(ts) == 1 else None
    # Match bytes: serial offsets count CRLF and UTF-8 bytes, not decoded
    # character offsets or Python's newline-normalized text positions.
    for match in re.finditer(rb'\[(SCHED|PMM-DIAG)\] (?:ipc_entries_reaped_for_process )?pid=(\d+) (?:cleared_msgs|owned_frames)=\d+[^\n]*\n', raw):
        pid = int(match[2])
        first_after = bisect.bisect_left(offsets, match.end())
        if not 0 < first_after < len(rows):
            continue
        if match[1] == b'SCHED':
            starts[pid] = first_after
        elif pid in starts:
            start, end = starts.pop(pid), first_after
            # [start-1,end] contains both boundaries; [start,end-1] is
            # wholly within the operation when the samples do not overlap.
            # Counters are read before the serial size within each sample.
            # Advance one extra sample at the end to enclose that read skew.
            if end + 1 >= len(rows):
                continue
            outer_a, outer_b = rows[start-1], rows[end+1]
            ca, cb = counters(outer_a), counters(outer_b)
            if ca is None or cb is None or ca['tid'] != cb['tid']:
                continue
            item = dict(pid=pid, host_begin_ns=outer_a['begin_ns'], host_end_ns=outer_b['end_ns'],
                        wall_upper_ns=outer_b['end_ns']-outer_a['begin_ns'],
                        runtime_upper_ns=cb['runtime_ns']-ca['runtime_ns'],
                        runqueue_wait_upper_ns=cb['runqueue_wait_ns']-ca['runqueue_wait_ns'],
                        wall_lower_ns=0, runtime_lower_ns=0, runqueue_wait_lower_ns=0)
            if start + 1 < end:
                ia, ib = rows[start+1], rows[end-1]
                a, b = counters(ia), counters(ib)
                if a and b and a['tid'] == b['tid']:
                    item.update(wall_lower_ns=max(0, ib['begin_ns']-ia['end_ns']),
                                runtime_lower_ns=max(0, b['runtime_ns']-a['runtime_ns']),
                                runqueue_wait_lower_ns=max(0, b['runqueue_wait_ns']-a['runqueue_wait_ns']))
            result.append(item)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('serial', type=Path)
    parser.add_argument('--mixed', action='store_true')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    raw = args.serial.read_text(errors='replace')
    measured = analyze(raw, args.mixed)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / 'timer.json').write_text(json.dumps(measured, indent=2) + '\n')
    all_samples, all_gaps, all_checkpoints, all_tails, trials, all_notifications = [], [], [], [], [], []
    all_rpc, rpc_tails = [], []
    for run, metric in zip(frames(raw), measured['runs']):
        common = {k: metric[k] for k in ('run', 'startup', 'duration_ms', 'order')}
        ss, gg, cc, tt = samples(run)
        all_samples += [dict(**common, **v) for v in ss]
        all_gaps += [dict(**common, **v) for v in gg]
        all_checkpoints += [dict(**common, **v) for v in cc]
        all_tails += [dict(**common, **v) for v in tt]
        ns = notifications(run)
        all_notifications += [dict(**common, **v) for v in ns]
        for reply in metric['replies']:
            all_rpc.append(dict(**common, **reply))
            if reply['complete'] - reply['request'] >= 300_000_000:
                rpc_tails.append(dict(**common, sample=reply,
                    events=[e for e in run['events'] if reply['request'] <= e[1] <= reply['complete']]))
        completed = [v for v in ss if v['kind'] == 'wake' and v['status'] == 'complete']
        trials.append(dict(**common, wake=quantiles([v['ns'] for v in completed]),
                           censored_wakes=sum(v['kind'] == 'wake' and v['status'] != 'complete' for v in ss),
                           multiple_decisions=sum(v['decisions'] > 1 for v in completed),
                           irq_entries=sum(e[0] == 1 for e in run['events']),
                           publications=sum(e[0] == 2 for e in run['events']),
                           tick_receives=sum(e[0] == 6 and e[4] == 0 for e in run['events']),
                           coalesced_completed_notifications=sum(v['status'] == 'complete' and v['publications'] > 1 for v in ns),
                           cpu_jain=metric['cpu_jain'], ops_per_second=metric['ops_per_second'],
                           wake_nice_counts=metric['wake_nice_counts']))
    def write_csv(name, records):
        if not records:
            return
        with (args.output / name).open('w') as stream:
            writer = csv.DictWriter(stream, fieldnames=list(dict.fromkeys(k for row in records for k in row)))
            writer.writeheader()
            writer.writerows(records)
    write_csv('samples.csv', all_samples)
    write_csv('irq-gaps.csv', all_gaps)
    write_csv('checkpoints.csv', all_checkpoints)
    write_csv('notifications.csv', all_notifications)
    write_csv('rpc-samples.csv', all_rpc)
    (args.output / 'tails.json').write_text(json.dumps(all_tails, indent=2) + '\n')
    (args.output / 'rpc-tails.json').write_text(json.dumps(rpc_tails, indent=2) + '\n')
    summary = dict(trials=trials, completed_samples=sum(v['status'] == 'complete' for v in all_samples),
                   censored_samples=[v for v in all_samples if v['status'] != 'complete'],
                   long_checkpoints=sorted(all_checkpoints, key=lambda v: v['ns'], reverse=True)[:40])
    host = args.serial.with_name(args.serial.name.replace('.serial.log', '.host.jsonl'))
    if host.exists():
        summary['host'] = host_summary(host)
        write_csv('host-reaps.csv', host_reaps(host, args.serial.read_bytes()))
    (args.output / 'investigation.json').write_text(json.dumps(summary, indent=2) + '\n')
    for startup in (True, False):
        wakes = [v for v in all_samples if v['startup'] == startup and v['kind'] == 'wake' and v['status'] == 'complete']
        print(json.dumps(dict(startup=startup, wake=quantiles([v['ns'] for v in wakes]),
                              multiple_decisions=sum(v['decisions'] > 1 for v in wakes))))


if __name__ == '__main__':
    main()
