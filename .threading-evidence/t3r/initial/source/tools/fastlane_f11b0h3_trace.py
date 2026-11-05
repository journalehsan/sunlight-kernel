#!/usr/bin/env python3
"""Read the H3 bounded trace as observations, never as security acceptance."""
import argparse
import json
import re
from pathlib import Path

PREFIX = '[H3-TRACE] '

def assess(raw):
    rows = []
    header = None
    errors = []
    for line in raw.splitlines():
        if not line.startswith(PREFIX):
            continue
        fields = dict(re.findall(r'([a-z]+)=([0-9]+)', line[len(PREFIX):]))
        if 'count' in fields:
            if header is not None:
                errors.append('duplicate header')
            header = fields
        elif 'seq' in fields:
            rows.append(fields)
    if not header:
        return {'classification': 'NOT_RUN', 'errors': ['trace dump missing']}
    count = int(header.get('count', -1))
    capacity = int(header.get('capacity', -1))
    if header.get('schema') != '1' or capacity != 256:
        errors.append('schema or capacity mismatch')
    if header.get('overflow') != '0' or count > capacity:
        errors.append('trace overflow')
    if len(rows) != count or [int(r.get('seq', 0)) for r in rows] != list(range(1, count + 1)):
        errors.append('missing, duplicate, or unordered slot')
    if any(r.get('schema') != '1' or int(r.get('event', 0)) == 0 for r in rows):
        errors.append('unpublished or invalid event')
    arms = [r for r in rows if r.get('event') == '1']
    if len(arms) != 1 or not all(int(arms[0].get(k, '0')) for k in ('pid', 'inc', 'b', 'c')):
        errors.append('authenticated fixture identity missing')
    if errors:
        return {'classification': 'FAIL_CURRENT', 'errors': errors}
    paths = [r for r in rows if r['event'] == '10']
    closes = [r for r in rows if r['event'] == '30']
    retire = [r for r in rows if r['event'] == '20']
    commits = [r for r in rows if r['event'] in ('4', '5')]
    return {'classification': 'OBSERVATIONS_ONLY', 'errors': [], 'case': arms[0]['a'],
            'path_events': paths, 'peer_call_closed': closes, 'session_retired': retire,
            'grant_outcomes': commits, 'contention': 'NOT_FORCED',
            'reply_publication': 'NOT_FORCED', 'security_acceptance': 'NOT_FORCED'}

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('serial', type=Path)
    args = parser.parse_args()
    result = assess(args.serial.read_text(errors='replace'))
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result['classification'] == 'OBSERVATIONS_ONLY' else 1)
