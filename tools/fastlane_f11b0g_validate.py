#!/usr/bin/env python3
"""G closure extends the F collector and census; no independent runner."""
import re
from fastlane_f11b0f_validate import main, fields, GATES
from fastlane_f11b0e3_validate import ROOT


def assess_g(raw, cpus, identity=None, *, transport=False):
    errors = []
    records = []
    for position, line in enumerate(raw.splitlines()):
        if '[GRANT-G] ' in line or '[HISTORY-G] ' in line or '[HISTORY-GT] ' in line:
            pairs = re.findall(r'(\w+)=(\S+)', line)
            row = fields(line)
            if len(row) != len(pairs): errors.append('duplicate marker fields')
            records.append({**row, 'marker': line.split('] ', 1)[1].split()[0], 'position': position})
    def rows(marker): return [r for r in records if r['marker'] == marker]
    try:
        if not identity or not all(identity.get(k) is True for k in ('source_unchanged', 'iso_matches', 'elf_matches', 'embedded_match')):
            errors.append('missing verified source/image identity')
        if any(s in raw for s in ('PANIC', 'panicked at', 'client FAIL')): errors.append('native failure')
        allowed = {'service-ack','boundary','exec-reply','cycle-commit','cycles','armed','ready','attempt','COMPLETE','prefill','rollback','client'}
        if any(r['marker'] not in allowed for r in records): errors.append('unknown closure checkpoint')
        acks, bounds = rows('service-ack'), rows('boundary')
        for r in acks:
            if any(not isinstance(r[k], int) or r[k] < 0 for k in ('records','active_or_boot_handles','heap_used')): errors.append('invalid service inventory')
        if len({r['caller_inc'] for r in acks}) != 1: errors.append('controller incarnation changed')
        if len(acks) != 8 or len(bounds) != 4: errors.append('missing/duplicate quiescence observations')
        for epoch in range(1, 5):
            aa = [r for r in acks if r['epoch'] == epoch]
            bb = [r for r in bounds if r['epoch'] == epoch]
            if [r['service'] for r in aa] != ['uac', 'vfs'] or len(bb) != 1: errors.append('service operation set mismatch'); continue
            if not aa[0]['position'] < aa[1]['position'] < bb[0]['position']: errors.append('snapshot before service drain')
            if any(min(r[k] for k in ('pid','incarnation','vm','caller_inc','request')) <= 0 or r['caller'] != 0xF4F0 for r in aa): errors.append('invalid authenticated service identity')
            if bb[0]['scope'] != 'completed-fixture-operations' or bb[0]['sessiond'] != 'UNMEASURED' or bb[0]['fixture_grants'] != 0: errors.append('unsupported reconciliation scope')
        for service in ('uac','vfs'):
            ss = [r for r in acks if r['service'] == service]
            if len({(r['pid'], r['incarnation'], r['vm']) for r in ss}) != 1: errors.append('service incarnation changed during drain')
        ee = rows('exec-reply')
        if [e['checkpoint'] for e in ee] not in (['commit','cancel','exec','reaped','late-settlement'], ['commit','cancel','exec','late-settlement','reaped']): errors.append('missing/wrong exec versus reply ordering')
        if any(min(e['pid'], e['incarnation']) <= 0 for e in ee): errors.append('invalid exec recipient identity')
        if len({(e['pid'],e['incarnation']) for e in ee}) != 1: errors.append('exec reply recipient mismatch')
        commits = [e for e in ee if e['checkpoint'] == 'commit']
        cancelled = [e for e in ee if e['checkpoint'] == 'cancel']
        late = [e for e in ee if e['checkpoint'] == 'late-settlement']
        if len(commits) != 1 or len(cancelled) != 1 or len(late) != 1: errors.append('missing exact pending delivery terminal transition')
        else:
            c, x, l = commits[0], cancelled[0], late[0]
            if c['pid'] != 0xF4F5 or c['count'] != 2 or min(c['vm'],c['request']) <= 0: errors.append('invalid pending commit')
            if x['request'] != c['request'] or x['vm'] != c['vm'] or x['retired'] != 2 or x['inventory'] != 0 or x['terminal'] != 'deadline': errors.append('deadline did not retire exact committed operation')
            if l['request'] != c['request'] or l['inventory'] != 0: errors.append('late settlement operation mismatch')
        cc = rows('cycle-commit')
        if [r['epoch'] for r in cc] != [2,3,4] or any(r['pid'] != 0xF4F0 or r['count'] != 2 or min(r['incarnation'],r['vm'],r['request']) <= 0 for r in cc): errors.append('wrong repeated lifecycle operation identities')
        xx = [e for e in ee if e['checkpoint'] == 'exec']
        if len(xx) != 1 or xx[0]['old_vm'] == xx[0]['new_vm'] or xx[0]['retired'] != 2 or xx[0]['exec_retired'] != 0 or xx[0]['retirement'] != 'ipc-cancel-before-exec': errors.append('VM authority retirement unproven')
        if len(rows('cycle-commit')) != 3 or raw.count('[GRANT-G] cycles COMPLETE epochs=4 real_cycles=3 service_acks=8') != 1: errors.append('repeated real lifecycle cycles incomplete')
        if transport:
            if raw.count('[HISTORY-GT] COMPLETE PASS ') != 1: errors.append('missing successful transport terminal checkpoint')
            pp = rows('prefill')
            if len(pp) != 1 or pp[0]['capacity'] != 1024 or pp[0]['authority_added'] != 0 or pp[0]['entropy'] != 'production': errors.append('invalid transport setup')
            tt = rows('rollback')
            if len({(r['pid'],r['incarnation'],r['vm']) for r in tt}) != 1 or any(r['pid'] != 0xF4F0 or min(r['incarnation'],r['vm'],r['request']) <= 0 for r in tt): errors.append('rollback authority identity mismatch')
            if len(tt)==2 and tt[0]['request'] >= tt[1]['request']: errors.append('repeated failed issuance not a new request')
            if len(rows('prefill')) != 1 or rows('prefill')[0]['history'] != 1023: errors.append('missing exact partial-batch boundary')
            if [t['attempt'] for t in tt] != [1,2] or any(t['history'] != 1024 or t['vfs_partial'] != 0 or t['proof'] != 'retained' or t['error'] != 'TokenHistoryFull' for t in tt): errors.append('failed batch did not roll back')
            if raw.count('[HISTORY-GT] client error=TokenHistoryFull caps=0') != 2: errors.append('explicit exhaustion not transported to client')
            done = rows('COMPLETE')
            if len(done) != 1 or done[0]['history'] != 1024 or done[0]['attempts'] != 2 or done[0]['proof'] != 'exec-retired' or done[0]['error_transport'] != 'explicit': errors.append('exhausted lifecycle cleanup incomplete')
        else:
            if raw.count('[HISTORY-G] COMPLETE PASS ') != 1: errors.append('missing successful exhaustion terminal checkpoint')
            arm, attempts, ready, done = rows('armed'), rows('attempt'), rows('ready'), rows('COMPLETE')
            if len(arm) != 1 or len(done) != 1: errors.append('missing/duplicate global exhaustion boundary')
            else:
                a, d = arm[0], done[0]
                if a['capacity'] != 1024 or a['production_capacity'] != 65536 or a['history'] != 1023 or a['cpus'] != cpus or a['entropy'] != 'production' or a['scope'] != 'global-production-broker' or a['record_bytes'] != 8 or a['identity_domain'] != 'kernel-boot' or a['reset'] != 'UNSUPPORTED' or a['sentinel'] != 'usable' or a['oldest'] != 'denied': errors.append('wrong exhaustion configuration')
                if len(ready) != cpus or {r['cpu'] for r in ready} != set(range(cpus)) or len(attempts) != cpus or {r['cpu'] for r in attempts} != set(range(cpus)): errors.append('missing/duplicate native CPU participants')
                if ready and attempts and max(r['position'] for r in ready) >= min(r['position'] for r in attempts): errors.append('issuance before all native arrivals')
                if any(r['locks'] != 'none' for r in ready): errors.append('rendezvous retained locks')
                if any(r.get('partial') != 0 for r in attempts if r['result'] == 'TokenHistoryFull'): errors.append('failed issuance published partial authority')
                if any(r.get('newest') != 'denied' for r in attempts if r['result'] == 'last-slot'): errors.append('newest replay unproven')
                if sum(r['result'] == 'last-slot' for r in attempts) != 1 or sum(r['result'] == 'TokenHistoryFull' for r in attempts) != cpus-1: errors.append('incorrect final-slot winners')
                if d['ready_mask'] != (1<<cpus)-1 or d['done_mask'] != (1<<cpus)-1 or d['winners'] != 1 or d['rejected'] != cpus-1: errors.append('incorrect native rendezvous completion')
                if d['history'] != 1024 or d['capacity'] != 1024 or d['retained_bytes'] != 8192 or d['final_live'] != a['baseline_live']: errors.append('history/authority inventory drift')
                if any(d[k] != v for k,v in [('oldest','denied'),('newest','denied'),('existing','retired'),('rollback','zero-partial'),('recovery','none'),('syscall_error_transport','NOT_RUN')]): errors.append('missing replay/rollback or unsupported coverage claim')
                if done[0]['position'] <= max((r['position'] for r in attempts), default=10**9): errors.append('completion before attempts')
        if any('cpu' in r and not 0 <= r['cpu'] < cpus for r in records): errors.append('invalid CPU')
    except (KeyError, TypeError, ValueError) as error:
        errors.append('incomplete checkpoint fields: '+str(error))
    return {'classification': 'FORCED_AND_FAILED' if errors else 'FORCED_AND_PASSED', 'errors': errors,
            'records': records, 'global_reconciliation': 'PARTIAL', 'sessiond_account_staging': 'UNMEASURED',
            'token_history': 'PARTIAL', 'syscall_error_transport': 'FORCED_AND_PASSED' if transport and not errors else 'NOT_RUN',
            'exec_before_grant_reply': 'NOT_FORCED' if errors else 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT'}


def assess_g_races(raw, cpus):
    from fastlane_f11b0d_validate import assess_races
    # The D parser's contract has exactly two memory/table boundaries. G adds
    # four intermediate observations; retain their raw evidence in the G result.
    scoped = "\n".join(line for line in raw.splitlines()
        if not (('[GRANT-D] accounting phase=cycle ' in line or '[GRANT-D] tables phase=cycle ' in line)))
    return assess_races(scoped, cpus)


def consolidate():
    import json
    from datetime import datetime, timezone
    from fastlane_f11b0e3_validate import digest, sources
    base = ROOT/'target/fastlane-f11b0g-evidence'
    current = sources()
    attempts = []
    for p in sorted(base.rglob('validation-summary.json')):
        if p.parent == base: continue
        d = json.loads(p.read_text())
        attempts.append({'manifest': str(p.relative_to(ROOT)), 'sha256': digest(p),
            'phase': d.get('phase'), 'section': d.get('section'),
            'source_unchanged': d.get('source_unchanged',d.get('source_hashes_unchanged')),
            'matches_final_source': d.get('source_sha256') == current,
            'checks': d.get('checks', []), 'images': d.get('images', {}),
            'native': d.get('native', [])})
    old = json.loads((base/'historical-preservation.json').read_text())['artifacts']
    changed = [p for p,h in old.items() if not (ROOT/p).is_file() or digest(ROOT/p) != h]
    summary = {'phase':'F1.1B-0G', 'updated_utc':datetime.now(timezone.utc).isoformat(),
        'source_sha256':current, 'attempts':attempts,
        'historical_preservation':{'inventoried':len(old), 'changed_or_missing':changed},
        'gates':{**GATES, 'global_authority':'PARTIAL', 'grant_lifecycle':'PARTIAL'},
        'security_decision':'See engineering report and final acceptance; incomplete invariants keep F1.1B-0 BLOCKED',
        'availability_decision':'Finite per-boot issuance budget; no production traffic measurement or in-boot renewal',
        'remaining_blockers':['Account/sessiond staging and retired-session observation at a common operation boundary',
            'issuer exec/retirement versus VFS commit native ordering',
            'legitimate VFS replacement versus pending old-generation authority'],
        'independent_findings':['original N3M lookup failure unexplained','historical and later D3 failures unexplained',
            'prior N4 timeout preserved','Session Authority PARTIAL','Doorbell B PARTIAL'],
        'repository_safety':{'commit':False,'push':False,'destructive_git':False,'production_fastlane_enabled':False,'restricted_shm_enabled':False}}
    report = ROOT/'docs/ipc/fastlane-f11b0g-global-authority-closure.md'
    if report.is_file(): summary['report_sha256'] = digest(report)
    (base/'validation-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(base/'validation-summary.json')


if __name__ == '__main__':
    import sys
    if '--section' in sys.argv and sys.argv[sys.argv.index('--section')+1] == 'consolidate':
        consolidate()
        raise SystemExit(0)
    transport = '--transport' in sys.argv
    if transport: sys.argv.remove('--transport')
    main(base=ROOT/'target/fastlane-f11b0g-evidence', phase='F1.1B-0G', kernel_feature='grant_history_transport_test' if transport else 'grant_history_test',
         service_feature='production-history-test' if transport else 'production-grant-test',
         race_assessor=assess_g_races, native_assessor=lambda raw, cpu, identity: assess_g(raw, cpu, identity, transport=transport),
         terminal='[HISTORY-GT] COMPLETE PASS' if transport else '[HISTORY-G] COMPLETE PASS',
         gates={**GATES, 'global_authority':'PARTIAL', 'grant_lifecycle':'PARTIAL'})
