#!/usr/bin/env python3
"""Acceptance coverage ledger. Missing forced native races keep T2 PARTIAL.

This reads recorded evidence; it never turns host models or a generic native
pressure verdict into proof of a particular lifecycle interleaving.
"""
import argparse
import json
from pathlib import Path

RACES = ('final_global_slot', 'final_vm_slot', 'parent_exit', 'vm_teardown',
         'exec', 'immediate_child_exit', 'concurrent_reap_reuse', 'prepublication_cancel')
SUITES = ('admission', 't1', 'auth', 'linux', 'fp', 'std', 'pin-r2d', 'doorbell')

def assess(summary):
    missing=[]
    for cpu in (1,2,4,12):
        for suite in SUITES:
            if not summary.get('native_matrix',{}).get(str(cpu),{}).get(suite,{}).get('pass'):
                missing.append(f'native/{cpu}/{suite}')
    for check in ('production_release_build','production_release_boot','production_debug_build',
                  'baremetal_check','allocation_host','admission_host','descriptor_host','parser_controls'):
        if not summary.get(check,{}).get('pass'): missing.append(check)
    races=summary.get('forced_native_races',{})
    for case in RACES:
        row=races.get(case,{})
        if not (row.get('pass') and row.get('cpus',0)>=2
                and (row.get('classification')=='FORCED_AND_PASSED'
                     or case=='exec' and row.get('classification')=='NOT_PERMITTED_VERIFIED'
                        and row.get('concurrent_exec_permitted') is False)
                and row.get('result') and row.get('serial_sha256') and row.get('iso_sha256')):
            missing.append('forced_native_race/'+case)
    return {'status':'PARTIAL' if missing else 'COMPLETE','missing':missing}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('summary',type=Path)
    args=p.parse_args()
    result=assess(json.loads(args.summary.read_text()))
    print(json.dumps(result,indent=2))
    raise SystemExit(0 if result['status']=='COMPLETE' else 1)

if __name__=='__main__': main()
