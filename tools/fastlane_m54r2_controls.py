#!/usr/bin/env python3
"""Opt-in bounded adversarial evidence models over an immutable native receipt.

The kernel is never asked to release a frame early. Each mutation is saved next
to its actual rejection. These models test the acceptance boundary, not native
fault injection. The independent native leak fixture is separately required.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re

from fastlane_channel_validate import rows
from fastlane_mapping_assess import assess_mapping
from fastlane_scoped_census import assess_trace, foreign_hold


def controls(raw, cpus, out):
    out.mkdir(parents=True, exist_ok=False)
    baseline = assess_mapping(raw, cpus)
    hold = foreign_hold(raw)
    result = {'baseline': baseline['status'], 'foreign_hold': hold,
              'serial_sha256': hashlib.sha256(raw.encode()).hexdigest(),
              'scope': 'safe-receipt-model; native leak fixture recorded separately',
              'controls': [], 'status': 'FAIL'}
    if baseline['status'] != 'PASS' or hold['status'] != 'PASS':
        result['errors'] = baseline['errors'] + hold['errors']
        return result

    def check(name, mutated, reason, verdict='REJECT', trace=False):
        (out / (name + '.serial.model.log')).write_text(mutated)
        receipt = assess_mapping(mutated, cpus)
        receipt['errors'] += foreign_hold(mutated)['errors']
        receipt['status'] = 'FAIL' if receipt['errors'] else 'PASS'
        if trace:
            receipt['errors'] += assess_trace(mutated, required=True)['errors']
            receipt['status'] = 'FAIL' if receipt['errors'] else 'PASS'
        rejected = receipt['status'] == 'FAIL' and reason in receipt['errors']
        row = {'name': name, 'execution': 'SAFE_RECEIPT_MODEL', 'expected': verdict,
               'actual': verdict if rejected else 'UNEXPECTED',
               'gate_status': receipt['status'], 'errors': receipt['errors'],
               'expected_result_preserved': rejected}
        result['controls'].append(row)
        (out / (name + '.json')).write_text(json.dumps(row, indent=2) + '\n')

    fixture = baseline['objects'][0]
    free = fixture['physical_free'][0]
    lines = raw.splitlines()
    free_line = next(l for l in lines if l.startswith('[FL-FRAME] ') and f"seq={free['seq']} " in l)
    # The independently owned FREE and all global census observations survive.
    check('E1-fixture-leak-foreign-free-present', raw.replace(free_line + '\n', '', 1),
          'missing/duplicate final physical free', 'FAIL')
    mapping = fixture['mappings'][0]['mapping']
    map_line = next(l for l in lines if l.startswith('[FL-MAP] ') and f"seq={mapping['seq']} " in l)
    for key in ('generation', 'pid', 'inc', 'vm'):
        changed = re.sub(rf'\b{key}=\d+', f'{key}={2**63}', map_line)
        check('E2-stale-' + key, raw.replace(map_line, changed, 1),
              'unaccounted mapping' if key == 'generation' else 'mapping lacks authenticated exact role binding')
    last_seq = max(int(r['seq']) for prefix in ('[FL-FRAME] ','[FL-MAP] ','[FL-TLB] ') for r in rows(raw,prefix))
    extra = re.sub(r'\bseq=\d+', f'seq={last_seq+1}', free_line)
    check('E3-duplicate-retirement', raw + '\n' + extra + '\n', 'missing/duplicate final physical free')
    wrong = re.sub(r'\bgeneration=\d+', 'generation=9223372036854775808', free_line)
    check('E3-wrong-lifetime-retirement', raw.replace(free_line, wrong, 1), 'missing/duplicate final physical free')
    trace_rows=rows(raw,'[FL-OWN] ')
    fb=baseline['ownership_trace']['fixture_bindings']['bindings'][0]
    drop=next(e for e in trace_rows if e['kind']=='9' and e['x1']==fb['shm_object'] and e['x3']=='0')
    transitions=[e for e in trace_rows if e['kind'] in ('1','2','3') and e['x0']==fb['frame'] and int(e['seq'])>int(drop['seq'])]
    if len(transitions)>1:
        second=transitions[1]
        old_line=next(l for l in lines if l.startswith('[FL-OWN] ') and f"seq={second['seq']} " in l)
        changed=re.sub(r'\bkind=\d+','kind=2',old_line)
        changed=re.sub(r'\bx1=\d+','x1=8',changed)
        changed=re.sub(r'\bx2=\d+',f"x2={drop['x2']}",changed)
        check('E3-duplicate-PMM-retirement-before-reuse',raw.replace(old_line,changed,1),
              'diagnostic fixture premature/duplicate physical retirement')
    else:
        result['controls'].append({'name':'E3-duplicate-PMM-retirement-before-reuse','actual':'NOT_FORCED',
                                   'expected_result_preserved':False})

    remote = next((o for o in baseline['objects'] if any(
        t.get('event') == 'request' and int(t['targets']) > 0
        for m in o['mappings'] for t in m['tlb'])), None)
    if remote is None:
        result['errors'] = ['E4 actual remote target missing']
        return result
    # Full native per-object request -> actual ACK -> completion -> free chain.
    result['remote_ack_native'] = remote
    result['controls'].append({'name':'E4-actual-remote-ack', 'execution':'NATIVE_OBSERVATION',
                               'expected':'PASS', 'actual':'PASS', 'expected_result_preserved':True})
    rf = remote['physical_free'][0]
    rline = next(l for l in lines if l.startswith('[FL-FRAME] ') and f"seq={rf['seq']} " in l)
    ack = min(int(t['seq']) for m in remote['mappings'] for t in m['tlb'] if t.get('event') == 'ack')
    early = re.sub(r'\bseq=\d+', f'seq={ack-1}', rline)
    check('E5-premature-physical-retirement', raw.replace(rline, early, 1),
          'frame freed before unmap and ACK completion')
    result['premature_native_fault_injection'] = 'NOT_RUN: safe acceptance-boundary model; no unsafe kernel release'
    result['controls'].append({'name':'E6-foreign-hold-valid', 'execution':'NATIVE_OBSERVATION',
                               'expected':'PASS', 'actual':hold['status'], 'expected_result_preserved':hold['status']=='PASS'})
    check('E6-foreign-free-cannot-compensate', raw.replace(free_line + '\n', '', 1),
          'fixture lifetime outstanding at census', 'FAIL')
    missing = '\n'.join(l for l in lines if not l.startswith('[FL-CENSUS-TRACE-END] ')) + '\n'
    check('E7-missing-closure', missing, 'missing/duplicate diagnostic trace closure', 'NOT_QUALIFIED', True)
    no_trace = '\n'.join(l for l in lines if not l.startswith(('[FL-OWN] ','[FL-CENSUS-TRACE'))) + '\n'
    check('E7-missing-entire-trace', no_trace, 'missing/duplicate diagnostic trace closure', 'NOT_QUALIFIED', True)
    end_line = next(l for l in lines if l.startswith('[FL-CENSUS-TRACE-END] '))
    overflow = raw.replace(end_line, end_line.replace('overflow=0', 'overflow=1'), 1)
    check('E7-overflow', overflow, 'incomplete/overflowing diagnostic trace', 'NOT_QUALIFIED', True)
    first = next(l for l in lines if l.startswith('[FL-OWN] '))
    check('E7-inconsistent-sequence', raw.replace(first + '\n', '', 1),
          'missing/duplicate diagnostic sequence', 'NOT_QUALIFIED', True)
    trace_rows=rows(raw,'[FL-OWN] ')
    binding=baseline['ownership_trace']['fixture_bindings']['bindings'][0]
    fixture_create=next(e for e in trace_rows if e['kind']=='7' and e['x1']==binding['shm_object'])
    fc_line=next(l for l in lines if l.startswith('[FL-OWN] ') and f"seq={fixture_create['seq']} " in l)
    bad_owner=re.sub(r'\bx2=\d+', 'x2=9223372036854775808', fc_line)
    check('E7-contradictory-fixture-owner',raw.replace(fc_line,bad_owner,1),
          'diagnostic fixture backing owner mismatch','NOT_QUALIFIED',True)
    independent=next(e for e in hold['witnesses'] if e['kind']=='7')
    own_line=next(l for l in lines if l.startswith('[FL-OWN] ') and f"seq={independent['seq']} " in l)
    check('E7-contradictory-independent-owner',raw.replace(own_line,re.sub(r'\bx2=\d+','x2=9223372036854775808',own_line),1),
          'independent object owner/lifetime mismatch','NOT_QUALIFIED',True)
    result['status'] = 'PASS' if all(c['expected_result_preserved'] for c in result['controls']) else 'FAIL'
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', type=Path, required=True)
    parser.add_argument('--cpus', type=int, choices=(2,4,12), required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--native-manifest', type=Path, required=True)
    args = parser.parse_args()
    from fastlane_f11b0e_validate import sources, digest
    manifest = json.loads(args.native_manifest.read_text())
    source = sources()
    assert manifest['status']=='PASS' and manifest['source_sha256']==source, 'native source mismatch'
    matching=[r for r in manifest['native'] if r['cpus']==args.cpus and Path(r['serial']).resolve()==args.serial.resolve()]
    assert len(matching)==1 and matching[0]['assessment']['status']=='PASS', 'native receipt missing'
    assert digest(args.serial)==matching[0]['serial_sha256'], 'native serial mismatch'
    result = controls(args.serial.read_text(), args.cpus, args.out)
    result['serial'] = str(args.serial)
    result['native_manifest'] = str(args.native_manifest)
    result['source_sha256'] = source
    if sources()!=source:
        result['status']='FAIL';result['source_changed']=True
    (args.out/'controls.json').write_text(json.dumps(result,indent=2)+'\n')
    print(result['status'], args.out)
    return int(result['status'] != 'PASS')


if __name__ == '__main__':
    raise SystemExit(main())
