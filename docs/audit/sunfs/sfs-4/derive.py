#!/usr/bin/env python3
"""Derive source-bound schedule indexes and scoped request counts after qualification.

Uses emitted evidence only. Does not implement filesystem parsing or liveness.
Run after tools/sunfs4_validate.py --qualify and before --seal.
"""
import gzip
import hashlib
import json
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[3]
COUNTS_SOURCE_SHA = '9f473d7dbc2f373b9023ccb0473f863287cef8e117958752a0e2c1a4223faa5d'

def write(p, value):
    p.write_text(json.dumps(value, indent=2)+'\n')

def main():
    regression = json.loads((OUT/'regression-results.json').read_text())
    assert regression['status'] == 'PASS'
    assert regression['compiled_source_sha256']['sunlight-sunfs-lab/tests/cleaning.rs'] == COUNTS_SOURCE_SHA
    raw = OUT/regression['attempt']
    manifest = raw/'compiled-source.json'
    source_id = hashlib.sha256(manifest.read_bytes()).hexdigest()
    indexes = []
    for profile in ['dev', 'release']:
        rows = []
        for sub in ['sfs2', 'sfs3', 'sfs4']:
            for p in sorted((raw/profile/sub).glob('*.jsonl')):
                if not (p.stem.startswith('crash-') or p.stem == 'reordering'):
                    continue
                corpus = [json.loads(line) for line in p.read_text().splitlines()]
                initial = min(min(r['expected']) for r in corpus)
                bps = 256 if sub == 'sfs2' else 64
                for line, record in enumerate(corpus, 1):
                    generation = record.get('initial_generation', record['expected'][0] if p.stem == 'reordering' else initial)
                    rows.append(dict(source_manifest=str(manifest.relative_to(OUT)), source_manifest_sha256=source_id,
                        profile=profile, corpus=str(p.relative_to(OUT)), line=line,
                        geometry=dict(capacity_bytes=16*1024*1024, fs_block_bytes=4096, segment_blocks=bps,
                                      checkpoint_slots=3, cleaner_reserve=1063 if sub == 'sfs2' else 874),
                        initial_durable_generation=generation, schedule=record,
                        assertion_oracle='Rust integration suite: independent closure/checker/contents/ownership and accounting assertions; raw trace and before images retained',
                        test_source={'sfs2':'sunlight-sunfs-lab/tests/transactions.rs', 'sfs3':'sunlight-sunfs-lab/tests/namespace.rs', 'sfs4':'sunlight-sunfs-lab/tests/cleaning.rs'}[sub]))
        assert len(rows) == 13142, len(rows)
        p = raw/f'schedule-index-{profile}.jsonl.gz'
        p.write_bytes(gzip.compress(('\n'.join(json.dumps(r, sort_keys=True) for r in rows)+'\n').encode(), mtime=0))
        indexes.append(dict(profile=profile, path=str(p.relative_to(OUT)), schedules=len(rows), sha256=hashlib.sha256(p.read_bytes()).hexdigest()))
    crash = json.loads((OUT/'crash-matrix.json').read_text())
    crash['source_bound_schedule_indexes'] = indexes
    write(OUT/'crash-matrix.json', crash)
    performance = json.loads((OUT/'performance-baseline.json').read_text())
    # F2 and F3 intentionally share one operation and two assertions. An outcome
    # file is not a request counter. Keep observation and execution counts separate.
    observed = performance.pop('no_net_fixture_requests', performance.get('recorded_no_net_observations'))
    performance['recorded_no_net_observations'] = observed
    performance['bounded_fixture_requests'] = dict(total=52, positive_net_acknowledged=6,
        deferred_acknowledged_without_positive_net=3, no_io_refusals=41,
        source_verification_failures_without_persistent_io=2,
        basis='Fixed executed test loops, bound to COUNTS_SOURCE_SHA in derive.py; excludes f10 campaign setup, injected requests and recovery retries',
        per_test={'F1':4, 'F2_F3_shared':1, 'F4':1, 'F5':3, 'F6_F8_restored_admission':1,
                  'F7':12, 'F8_F9_churn':25, 'F12':1, 'inode_attributes_and_source_budget':3, 'checker_corruptions_setup':1})
    assert sum(performance['bounded_fixture_requests']['per_test'].values()) == 52
    rows = [json.loads(line) for p in (raw/'dev/sfs4').glob('crash-*.jsonl') for line in p.read_text().splitlines()]
    performance['injected_requests_per_profile'] = dict(total=len(rows), acknowledged=sum(r['acknowledged'] for r in rows),
        unacknowledged=sum(not r['acknowledged'] for r in rows),
        scope='Unique injected requests only; source-derived fixture counts, setup dry runs and recovery retries are separate; uncertain durable capacity is not claimed as acknowledged progress')
    write(OUT/'performance-baseline.json', performance)
    write(OUT/'evidence-derivations.json', dict(status='PASS', script='docs/audit/sunfs/sfs-4/derive.py',
        source_manifest_sha256=source_id, schedule_indexes=indexes,
        geometry_basis='Compiled fixture constructors: SFS2 committed_fixture fresh(256,16); SFS3 fixture(64) fresh(64,16); SFS4 fresh(64) 16 MiB. Default reserve formula gives 1063 and 874 blocks.',
        counting='Source-derived fixed fixture calls are operational measurements, not added fault coverage. F2/F3 is one request. Each profile indexes legacy 10182 plus new 2960. CPU replays are excluded.'))
    print('Derived source-bound indexes and scoped request counts PASS')

if __name__ == '__main__':
    main()
