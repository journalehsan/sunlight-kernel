#!/usr/bin/env python3
"""Build happens-before graphs and classify SFS-5B.5 native runs.

Edges are only added where an actual synchronization edge exists: per-thread
program order, a mutex/condvar handshake, a guest port or memory handshake, or
an explicit join.  Guest atomic log-counter order across CPUs is never treated
as task happens-before.
"""
import argparse
import json
from pathlib import Path

OUT = Path(__file__).resolve().parent

DEVICE = r'SFS5B5 seq=(\d+) ns=(-?\d+) thread=(\w+) stage=(\w+) iter=(\d+) bytes=(\d+) extra=(\d+)'
GUEST = (r'\[SFS5B4\] event=(\d+) cpu=(\d+) pid=(\d+) task-generation=(\d+) '
         r'role=(\d+) scheduler-ready=(\w+) stage=(\w+)')


def parse(raw, tag):
    import re
    dev = []
    qlog = (raw / (tag + '.qemu.log'))
    if qlog.exists():
        for a, b, c, d, e, f, g in re.findall(DEVICE, qlog.read_text(errors='replace')):
            dev.append(dict(kind='device', seq=int(a), ns=int(b), thread=c, stage=d,
                            iter=int(e), bytes=int(f), extra=int(g)))
    guest = []
    slog = (raw / (tag + '.serial.log'))
    if slog.exists():
        for a, b, c, d, e, f, g in re.findall(GUEST, slog.read_text(errors='replace')):
            guest.append(dict(kind='guest', seq=int(a), cpu=int(b), pid=int(c),
                              task=int(d), role=int(e), ready=f, stage=g))
    return dev, guest


def build(dev, guest):
    nodes = {}
    for e in dev:
        nodes[f'd{e["seq"]}'] = e
    for e in guest:
        nodes[f'g{e["seq"]}'] = e
    edges = []

    def add(a, b, kind):
        if a in nodes and b in nodes and a != b:
            edges.append(dict(**{'from': a, 'to': b, 'kind': kind}))

    def stage_nodes(prefix, stage):
        return [f'{prefix}{e["seq"]}' for e in (dev if prefix == 'd' else guest)
                if e['stage'] == stage]

    # 1/2: per-thread program order.
    for thread in ('main', 'worker'):
        seq = [f'd{e["seq"]}' for e in dev if e['thread'] == thread]
        for a, b in zip(seq, seq[1:]):
            add(a, b, f'device-{thread}-program-order')
    # 3: per-task guest program order.
    tasks = {}
    for e in guest:
        tasks.setdefault((e['pid'], e['task']), []).append(f'g{e["seq"]}')
    for seq in tasks.values():
        for a, b in zip(seq, seq[1:]):
            add(a, b, 'guest-task-program-order')

    # 4: explicit cross-boundary handshakes.
    for a in stage_nodes('d', 'GUEST_WITNESS_PUBLISHED'):
        for b in stage_nodes('g', 'DEVICE_ACCESS_HANDSHAKE'):
            add(a, b, 'handshake-status-byte')
    for a in stage_nodes('g', 'RESET_REQUESTED'):
        for b in stage_nodes('d', 'RESET_ENTERED'):
            add(a, b, 'handshake-status-zero-write')
    for a in stage_nodes('d', 'RESET_ACKNOWLEDGED'):
        for b in stage_nodes('g', 'RESET_ACKNOWLEDGED'):
            add(a, b, 'handshake-status-zero-readback')

    # 5: device-internal mutex/condvar and join edges.
    pairs = [
        ('RESET_ENTERED', 'ACCESS_OBSERVED_RESET_ACTIVE', 'mutex-reset-flag-publish'),
        ('ACCESS_OBSERVED_RESET_ACTIVE', 'ACCESS_ACTIVE_DURING_RESET', 'condvar-observe'),
        ('ACCESS_EXITED', 'CALLBACK_QUIESCED', 'condvar-worker-exit'),
        ('CALLBACK_QUIESCED', 'WORKER_JOINED', 'join'),
        ('WORKER_JOINED', 'MAPPING_RELEASED', 'join-before-unmap'),
        ('MAPPING_RELEASED', 'RESET_ACKNOWLEDGED', 'unmap-before-ack'),
        ('RESET_ACKNOWLEDGED', 'NEGATIVE_POST_ACK_ACCESS', 'negative-model-violation'),
    ]
    for a_stage, b_stage, kind in pairs:
        for a in stage_nodes('d', a_stage):
            for b in stage_nodes('d', b_stage):
                add(a, b, kind)
    return nodes, edges


def acyclic(nodes, edges):
    adj = {n: [] for n in nodes}
    for e in edges:
        adj[e['from']].append(e['to'])
    state = {}

    def visit(n):
        state[n] = 1
        for m in adj[n]:
            if state.get(m) == 1:
                return False
            if state.get(m) is None and not visit(m):
                return False
        state[n] = 2
        return True

    return all(state.get(n) or visit(n) for n in nodes)


def reaches(nodes, edges, a, b):
    adj = {n: [] for n in nodes}
    for e in edges:
        adj[e['from']].append(e['to'])
    seen, stack = set(), [a]
    while stack:
        n = stack.pop()
        if n == b:
            return True
        if n in seen:
            continue
        seen.add(n)
        stack.extend(adj.get(n, []))
    return False


def classify(raw, tag, mode):
    dev, guest = parse(raw, tag)
    nodes, edges = build(dev, guest)
    ds = [e['stage'] for e in dev]
    gs = [e['stage'] for e in guest]
    first = lambda seq, s: next((e for e in seq if e['stage'] == s), None)
    d_reset = first(dev, 'RESET_ENTERED')
    d_obs = first(dev, 'ACCESS_OBSERVED_RESET_ACTIVE')
    d_active = first(dev, 'ACCESS_ACTIVE_DURING_RESET')
    d_exit = first(dev, 'ACCESS_EXITED')
    d_quiet = first(dev, 'CALLBACK_QUIESCED')
    d_join = first(dev, 'WORKER_JOINED')
    d_unmap = first(dev, 'MAPPING_RELEASED')
    d_ack = first(dev, 'RESET_ACKNOWLEDGED')
    d_post = first(dev, 'NEGATIVE_POST_ACK_ACCESS')

    ack_node = f'd{d_ack["seq"]}' if d_ack else None
    post_ack_access = bool(d_ack) and any(
        e['stage'] in ('ACCESS_ENTERED', 'ACCESS_OBSERVED_RESET_ACTIVE',
                       'NEGATIVE_POST_ACK_ACCESS') and e['seq'] > d_ack['seq'] for e in dev)

    overlap = bool(d_reset and d_obs and d_active
                   and d_active['extra'] > 0
                   and d_obs['iter'] > d_reset['iter'])

    order_ok = bool(
        d_exit and d_quiet and d_join and d_unmap and d_ack
        and d_exit['seq'] < d_quiet['seq'] < d_join['seq']
        < d_unmap['seq'] < d_ack['seq'])

    common = dict(
        guest_handshake='DEVICE_ACCESS_HANDSHAKE' in gs,
        guest_timeout='TIMEOUT_QUARANTINE' in gs,
        guest_admission_closed='ADMISSION_CLOSED' in gs,
        guest_reset_ack='RESET_ACKNOWLEDGED' in gs,
        guest_terminal='TERMINAL_QUARANTINE' in gs,
        guest_progress='FORWARD_PROGRESS' in gs,
        device_access_entered='ACCESS_ENTERED' in ds,
        device_reset_entered='RESET_ENTERED' in ds,
        observed_reset_while_active=bool(d_obs),
        iterations_advanced_during_reset=overlap,
    )
    if mode == 'async-negative':
        # The deliberately broken model must be caught by the independent
        # observer: a real access after acknowledgment, a violated quiescence
        # order and therefore a cyclic happens-before graph.
        checks = dict(common)
        checks.update(
            negative_post_ack_access=post_ack_access,
            negative_quiescence_order_violated=not order_ok,
            negative_graph_cycle=not acyclic(nodes, edges),
        )
    else:
        checks = dict(common)
        checks.update(
            quiescence_order=order_ok,
            graph_acyclic=acyclic(nodes, edges),
            no_access_after_ack=not post_ack_access,
        )
    status = 'PASS' if all(checks.values()) else 'FAIL'
    return dict(
        tag=tag, mode=mode, status=status, checks=checks,
        overlap=dict(
            proven=overlap,
            iterations_at_reset=d_reset['iter'] if d_reset else None,
            iterations_observed_by_worker=d_obs['iter'] if d_obs else None,
            iterations_after_reset_wait=d_active['iter'] if d_active else None,
            advanced_during_reset=d_active['extra'] if d_active else None,
            reset_ns=d_reset['ns'] if d_reset else None,
            worker_observed_ns=d_obs['ns'] if d_obs else None,
            acknowledged_ns=d_ack['ns'] if d_ack else None,
            model='INSTRUMENTED_QEMU genuine worker thread, BQL main thread'),
        quiescence=dict(
            access_exited=d_exit['seq'] if d_exit else None,
            callback_quiesced=d_quiet['seq'] if d_quiet else None,
            worker_joined=d_join['seq'] if d_join else None,
            mapping_released=d_unmap['seq'] if d_unmap else None,
            reset_acknowledged=d_ack['seq'] if d_ack else None),
        post_ack_access=post_ack_access,
        nodes=len(nodes), edges=len(edges),
        graph=dict(nodes=list(nodes.values()), edges=edges))


def main():
    p = argparse.ArgumentParser()
    p.add_argument('raw')
    p.add_argument('--out', default=None)
    args = p.parse_args()
    raw = Path(args.raw)
    out = Path(args.out) if args.out else raw
    runs = json.loads((raw / 'native-runs.json').read_text())
    cases = []
    graphs = []
    for r in runs:
        c = classify(raw, r['tag'], r['mode'])
        c['ready'] = r['ready']
        c['elapsed_s'] = r['elapsed_s']
        c['cpu'] = r['cpu']
        c['cache'] = r['cache']
        if not r['ready']:
            c['status'] = 'FAIL'
            c['checks']['guest_reached_ready'] = False
        else:
            c['checks']['guest_reached_ready'] = True
        graphs.append(dict(tag=r['tag'], cpu=r['cpu'], cache=r['cache'], mode=r['mode'],
                           nodes=c['graph']['nodes'], edges=c['graph']['edges']))
        del c['graph']
        cases.append(c)
        print(r['tag'], c['status'],
              {k: v for k, v in c['checks'].items() if not v}, flush=True)
    status = 'PASS' if all(c['status'] == 'PASS' for c in cases) else 'PARTIAL'
    (out / 'async-access-witness.json').write_text(json.dumps(dict(
        status=status, cases=cases,
        true_simultaneous_access_reset=('PASS' if all(
            c['overlap']['proven'] for c in cases if c['mode'] == 'async') else 'UNQUALIFIED'),
        scope='instrumented QEMU 11.1.2 TCG, legacy PCI virtio-blk, split queue 0',
        not_claimed=['physical DMA engine', 'IOMMU fence', 'hardware persistence']), indent=2) + '\n')
    (out / 'independent-event-graph.json').write_text(json.dumps(dict(
        status='PASS' if all(acyclic({f'd{n["seq"]}' for n in g['nodes'] if n['kind'] == 'device'}
                                     | {f'g{n["seq"]}' for n in g['nodes'] if n['kind'] == 'guest'},
                                     g['edges']) for g in graphs) else 'FAIL',
        graphs=graphs), indent=2) + '\n')
    print('wrote', out, status)


if __name__ == '__main__':
    main()
