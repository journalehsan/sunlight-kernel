#!/usr/bin/env python3
"""Validate bounded C1.2E same-CPU traces; preserve completed and censored counts.

No cross-CPU subtraction and no conversion of dispatch counts to CPU service.
IRQ lateness is phase-relative observation, not an actual LAPIC expiry timestamp.
"""
import argparse
import json
from pathlib import Path
import re

from scheduler_fairness import percentile


def quantiles(values):
    return dict(n=len(values), **{f'p{p}_ms': percentile(values, p) / 1e6
                                 for p in (50, 95, 99, 100)}) if values else dict(n=0)


def frames(raw):
    runs = []
    current = None
    for line in raw.splitlines():
        m = re.fullmatch(r'\[SCHED-BENCH\] START (\{.*\})', line.strip())
        if m:
            if current is not None:
                raise ValueError('nested run')
            current = dict(start=json.loads(m[1]), events=[], workers=[], services=[], clients=[], audio=[], waits=[], samples=[], lifecycle=[])
            runs.append(current)
        m = re.fullmatch(r'\[(SCHED-BENCH|SCHED-TIMER)\] (\w+) ([\[{].*)', line.strip())
        if not m:
            continue
        if current is None:
            raise ValueError('record outside frame')
        kind, value = m[2], json.loads(m[3])
        if kind == 'TRACE':
            if 'trace' in current:
                raise ValueError('duplicate trace')
            current['trace'] = value
        elif kind in ('EVENT', 'WORKER', 'SERVICE', 'CLIENT', 'AUDIO', 'WAIT', 'SAMPLE', 'LIFECYCLE'):
            current[dict(EVENT='events', WORKER='workers', SERVICE='services', CLIENT='clients', AUDIO='audio', WAIT='waits', SAMPLE='samples', LIFECYCLE='lifecycle')[kind]].append(value)
        elif kind == 'END':
            current['end'] = value
            current = None
    if current is not None:
        raise ValueError('incomplete final run')
    return runs


def validate_run(run, mixed):
    start, end, trace = run['start'], run['end'], run['trace']
    ordinal = int(start['run'].split('-')[1])
    if end['run'] != start['run'] or end['status'] != 'ok' or not end['all_workers_progress']:
        raise ValueError('run did not complete successfully')
    if trace['run'] != ordinal or trace['overflow'] or trace['invalid_clock'] or trace['count'] != len(run['events']):
        raise ValueError('overflow, invalid clock or incomplete trace')
    rows = run['events']
    if not rows or any(len(e) != 7 or e[2] != 0 or any(type(x) is not int or x < 0 for x in e) for e in rows):
        raise ValueError('invalid event or CPU')
    if any(a[1] > b[1] for a, b in zip(rows, rows[1:])):
        raise ValueError('clock regression')
    expected_nice = [0, 8, 9, 10] if mixed and ordinal > 2 and ordinal % 2 == 0 else [0] * 4
    workers = sorted(run['workers'], key=lambda w: w['slot'])
    if len(workers) != 4 or [w['slot'] for w in workers] != list(range(4)):
        raise ValueError('missing/duplicate worker')
    for w, nice in zip(workers, expected_nice):
        if w['nice'] != nice or not w['reaped'] or w['invalid_observations'] or w['wait_overflow'] or w['cpu_ns'] <= 0 or w['dispatches'] <= 0:
            raise ValueError('worker invariant failed')
        waits = [v for v in run['waits'] if v['slot'] == w['slot']]
        if [v['index'] for v in waits] != list(range(w['dispatches'])) or len(waits) != w['wait_count']:
            raise ValueError('incomplete worker waits')
        if sum(v['ns'] for v in waits) != w['wait_ns'] or max(v['ns'] for v in waits) != w['wait_max_ns']:
            raise ValueError('worker wait aggregation failed')
    if len(run['samples']) != 1:
        raise ValueError('missing/duplicate sample')
    sample = run['samples'][0]
    if sample['run'] != start['run'] or sample['operations'] != end['operations'] or sum(sample[f'worker{s}'] for s in range(4)) != sample['operations'] or any(sample[f'worker{s}'] <= 0 for s in range(4)):
        raise ValueError('operation counts failed')
    if sample['elapsed_ms'] != end['elapsed_ms'] or end['elapsed_ms'] < start['duration_ms']:
        raise ValueError('elapsed time failed')
    life = sorted(run['lifecycle'], key=lambda l: l['slot'])
    if [v['slot'] for v in life] != list(range(4)):
        raise ValueError('incomplete lifecycle')
    for v in life:
        if not v['created_ms'] <= v['ready_ms'] <= v['start_ms'] <= v['entered_ms'] <= v['exited_ms'] or v['exited_ms'] < v['start_ms'] + start['duration_ms']:
            raise ValueError('lifecycle/barrier failed')
    if [s['name'] for s in run['services']] != ['timer_server', 'tty_server'] or any(s['overflow'] or not s['waits_ns'] for s in run['services']):
        raise ValueError('service trace failed')
    if mixed and (len(run['clients']) != 1 or run['clients'][0]['errors'] or run['clients'][0]['replies'] <= 0):
        raise ValueError('client failed')
    return ordinal, workers


def stages(run):
    series = {key: [] for key in ('irq_phase_late', 'irq_nominal_late', 'irq_interarrival', 'irq_to_publication', 'publication_to_ready',
        'ready_to_dispatch', 'external_ready_to_dispatch', 'internal_ready_to_dispatch', 'wake_to_dispatch',
        'publication_to_tick_receive', 'latest_publication_to_tick_receive', 'dispatch_to_tick_receive', 'service_wall', 'service_cpu',
        'request_to_receive', 'receive_to_reply', 'reply_service_cpu', 'reply_to_completion', 'request_to_completion')}
    entries, publications, ready, selected, requests, wakes = {}, {}, {}, {}, {}, {}
    replies, completed, censored_requests = [], set(), 0
    wake_nice_counts = {}
    origin = None
    previous_entry = None
    delivered_irqs = 0
    received = None
    pending_publication = None
    latest_publication = None
    pending_known = False
    sequence_offset = None
    latest_tick = None
    life = run.get('lifecycle', [])
    load_start_ms = max((v['entered_ms'] for v in life), default=0)
    load_end_ms = min((v['exited_ms'] for v in life), default=0)
    def saturated():
        # Classification only: use the guest's coarse 100 Hz monotonic domain
        # and leave a full tick margin at either edge. Durations still use TSC.
        return latest_tick is not None and load_start_ms + 10 <= latest_tick * 10 < load_end_ms - 10
    decisions = []
    waits = []
    def delta(key, end, begin):
        if end < begin:
            raise ValueError(f'negative {key}')
        series[key].append(end - begin)
    previous = None
    for kind, now, cpu, pid, a, b, c in run['events']:
        if kind == 1:
            latest_tick = b
            if a > now:
                raise ValueError('IRQ entry timestamp after observation')
            entries[b] = a
            if origin is None:
                origin = (a, b)
            if previous_entry is not None:
                delta('irq_interarrival', a, previous_entry)
            previous_entry = a
            series['irq_nominal_late'].append(max(0, a - (origin[0] + delivered_irqs * 10_000_000)))
            delivered_irqs += 1
            expected = origin[0] + (b - origin[1]) * 10_000_000
            series['irq_phase_late'].append(max(0, a - expected))
        elif kind == 2:
            publications[a] = now
            latest_publication = (a, now)
            if pending_publication is None:
                pending_publication = now
            if a in entries:
                delta('irq_to_publication', now, entries[a])
        elif kind == 3:
            ready.setdefault(pid, (now, a, b, c))
        elif kind == 4:
            selected[pid] = now
            if pid in wakes:
                delta('wake_to_dispatch', now, wakes.pop(pid))
            if pid in ready:
                since, notify, nice, outgoing = ready.pop(pid)
                if pid == run['services'][0]['pid']:
                    delta('ready_to_dispatch', now, since)
                    delta('external_ready_to_dispatch' if notify else 'internal_ready_to_dispatch', now, since)
                    waits.append(dict(ready_ns=since, dispatch_ns=now, notify=notify, nice=nice - 20, current=outgoing, reason=b))
        elif kind == 5:
            key = (a, b)
            if key in requests:
                raise ValueError('duplicate request identity')
            requests[key] = dict(request=now, request_saturated=saturated())
        elif kind == 6:
            received = (now, c)
            if a:
                if (a, b) in requests:
                    requests[a, b].update(receive=now, cpu=c)
            else:
                # Timer payload is endpoint-relative (latest-base+1), whereas
                # IRQ ticks are global. Verify the constant offset before
                # associating the latest coalesced publication with reception.
                if latest_publication:
                    offset = latest_publication[0] - b
                    if sequence_offset is not None and sequence_offset != offset:
                        raise ValueError('timer sequence/global tick association changed')
                    sequence_offset = offset
                    delta('latest_publication_to_tick_receive', now, latest_publication[1])
                if pending_known and pending_publication is not None:
                    delta('publication_to_tick_receive', now, pending_publication)
                pending_publication = None
                pending_known = True
                if pid in selected:
                    delta('dispatch_to_tick_receive', now, selected[pid])
        elif kind == 7:
            if (a, b) in requests:
                requests[a, b].update(reply=now, reply_cpu=c)
        elif kind == 8:
            key = (pid, a)
            if key in requests:
                req = requests[key]
                if key in completed or any(k not in req for k in ('receive', 'reply')):
                    raise ValueError('missing/duplicate IPC stage')
                completed.add(key)
                delta('request_to_receive', req['receive'], req['request'])
                delta('receive_to_reply', req['reply'], req['receive'])
                delta('reply_service_cpu', req['reply_cpu'], req['cpu'])
                delta('reply_to_completion', now, req['reply'])
                delta('request_to_completion', now, req['request'])
                replies.append(dict(pid=pid, generation=a, **req, complete=now, complete_saturated=saturated()))
        elif kind == 9:
            decisions.append(dict(at=now, timer=pid, current=a, nice=b - 20, flags=c))
        elif kind == 10 and received:
            delta('service_wall', now, received[0])
            delta('service_cpu', c, received[1])
            received = None
        elif kind == 11:
            wakes.setdefault(pid, now)
            wake_nice_counts[a - 20] = wake_nice_counts.get(a - 20, 0) + 1
            if previous and previous[0] == 2 and previous[3] == pid:
                delta('publication_to_ready', now, previous[1])
                pending_known = True
        previous = (kind, now, cpu, pid, a, b, c)
    censored_requests = len(requests) - len(completed)
    saturated_replies = [r for r in replies if r['request_saturated'] and r['complete_saturated']]
    return dict(series=series, quantiles={k: quantiles(v) for k, v in series.items()},
                saturated_rpc_quantiles=quantiles([r['complete'] - r['request'] for r in saturated_replies]),
                waits=waits, decisions=decisions, replies=replies,
                censored_ready=len(ready), censored_wakes=len(wakes), censored_requests=censored_requests,
                wake_nice_counts=wake_nice_counts, tick_sequence_offset=sequence_offset)


def analyze(raw, mixed=False):
    runs = frames(raw)
    count = 8 if mixed else 22
    marker = f'[SCHED-TIMER] MIXED complete runs={count}' if mixed else f'[SCHED-BENCH] SUITE complete runs={count}'
    if marker not in raw or len(runs) != count:
        raise ValueError('incomplete suite')
    result = []
    for index, run in enumerate(runs, 1):
        ordinal, workers = validate_run(run, mixed)
        if ordinal != index:
            raise ValueError('run order mismatch')
        stage = stages(run)
        if mixed and stage['censored_requests']:
            raise ValueError('incomplete timer RPC')
        if mixed and len(stage['replies']) != run['clients'][0]['replies']:
            raise ValueError('client/trace RPC count mismatch')
        cpu = [w['cpu_ns'] for w in workers]
        operations = [run['samples'][0][f'worker{slot}'] for slot in range(4)]
        result.append(dict(run=ordinal, startup=ordinal <= (3 if mixed else 4), duration_ms=run['start']['duration_ms'],
            order=run['start']['order'], nice=[w['nice'] for w in workers], cpu_ns=cpu,
            cpu_jain=sum(cpu)**2 / (4 * sum(x*x for x in cpu)),
            work_jain=sum(operations)**2 / (4 * sum(x*x for x in operations)), operations=operations,
            ops_per_second=run['end']['operations'] * 1000 / run['end']['elapsed_ms'],
            ops_per_worker_cpu_second=run['end']['operations'] * 1e9 / sum(cpu),
            worker_dispatches=sum(w['dispatches'] for w in workers),
            timer_cpu_ns=run['services'][0]['cpu_ns'], timer_dispatches=run['services'][0]['dispatches'],
            client=run['clients'], audio=run['audio'], **stage))
    if any(a['events'][-1][1] > b['events'][0][1] for a, b in zip(runs, runs[1:])):
        raise ValueError('clock regression between trials')
    if len({r['tick_sequence_offset'] for r in result if r['tick_sequence_offset'] is not None}) != 1:
        raise ValueError('timer sequence offset changed between trials')
    groups = []
    for startup in (True, False):
        for nice_test in (False, True) if mixed else (False,):
            for duration in (1000, 5000, 10000):
                for order in ('forward', 'reverse'):
                    members = [r for r in result if r['startup'] == startup and (r['nice'] != [0]*4) == nice_test and r['duration_ms'] == duration and r['order'] == order]
                    if members:
                        groups.append(dict(startup=startup, nice_test=nice_test, duration_ms=duration, order=order,
                            runs=[r['run'] for r in members], cpu_jain_mean=sum(r['cpu_jain'] for r in members) / len(members),
                            ops_per_second_mean=sum(r['ops_per_second'] for r in members) / len(members),
                            quantiles={k: quantiles([v for r in members for v in r['series'][k]]) for k in members[0]['series']}))
    return dict(runs=result, groups=groups)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('serial', type=Path)
    p.add_argument('--mixed', action='store_true')
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    result = analyze(args.serial.read_text(errors='replace'), args.mixed)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    for group in result['groups']:
        print(json.dumps(group))


if __name__ == '__main__':
    main()
