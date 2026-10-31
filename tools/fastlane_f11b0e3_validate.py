#!/usr/bin/env python3
"""Authenticated production session evidence; boot alone never certifies races."""
import argparse
from datetime import datetime, timezone
import json
import os
import secrets
from pathlib import Path
import re
import shutil
import subprocess
import time

from fastlane_f11b0e_validate import ROOT, digest, sources
from fastlane_f11b0c_validate import SERVICE_FLAGS
from voice_recorder_qmp import Qmp

BASE = ROOT / 'target/fastlane-f11b0e3-evidence'
BUILD = ROOT / 'target/fastlane-f11b0e3-build'
ENV = {'CARGO_TARGET_DIR': str(BUILD), 'CARGO_INCREMENTAL': '0', 'CARGO_PROFILE_DEV_DEBUG': '0'}
CASES = ('N1', 'N2', 'N3M', 'N3R', 'N4', 'N5', 'N6', 'N7R', 'N7C', 'N7OC', 'N7OR', 'H4R', 'S1', 'S2', 'S3')
ACCEPTED = ('FORCED_AND_PASSED', 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT')


def audit_attempt_inventory(recorded, indexed):
    """An aggregate must retain every raw attempt, including failed N3M runs."""
    expected = {r['serial']: r for r in recorded if r.get('serial')}
    actual = {r['serial']: r for r in indexed if r.get('serial')}
    errors = []
    if len(actual) != sum(bool(r.get('serial')) for r in indexed):
        errors.append('duplicate attempt in aggregate')
    if set(expected) != set(actual):
        errors.append('attempt omitted or unrecorded attempt added')
    for serial in set(expected) & set(actual):
        for key in ('case', 'cpus', 'failure', 'serial_sha256'):
            if expected[serial].get(key) != actual[serial].get(key):
                errors.append('attempt metadata changed: ' + serial + ':' + key)
        if expected[serial].get('validator', {}).get('classification') != actual[serial].get('validator', {}).get('classification'):
            errors.append('attempt classification changed: ' + serial)
    return errors


def assess_native(raw, case, cpus, *, identity=None, failure=None):
    """Raw markers alone cannot certify a native execution or its artifacts."""
    try:
        result = _assess_native(raw, case, cpus)
    except (ValueError, KeyError, TypeError):
        result = {'classification': 'FORCED_AND_FAILED', 'case': case, 'cpus': cpus,
                  'errors': ['malformed required marker field'], 'missing_checkpoints': []}
    result['identity'] = identity
    if identity is None or not all(identity.get(k) for k in
            ('iso_matches', 'elf_matches', 'services_match', 'source_matches')):
        result['errors'].append('missing or mismatched image/source identity')
        if result['classification'] in ACCEPTED:
            result['classification'] = 'FORCED_AND_FAILED'
    if failure:
        result['errors'].append('driver failed or timed out: ' + failure['type'])
        result['classification'] = 'FORCED_AND_FAILED'
    return result


def _assess_native(raw, case, cpus):
    """Fail closed on absent boundaries, identities, ordering or cleanup."""
    def parse(prefix):
        result = []
        for line in raw.splitlines():
            if line.startswith(prefix):
                pairs = re.findall(r'(\w+)=(\S+)', line)
                row = dict(pairs)
                if len(row) != len(pairs): row['_duplicate_fields'] = '1'
                result.append(row)
        return result
    rows = parse('[SESSION-E3] ')
    clients = [r for r in parse('[SESSION-E3-CLIENT] ') if r.get('case') == case]
    uac = parse('[SESSION-E3-UAC] ')
    errors, missing = [], []
    def events(kind): return [r for r in rows if r.get('checkpoint') == kind]
    def client(kind): return [r for r in clients if r.get('checkpoint') == kind]
    def ordered(a, b): return bool(a and b and int(a[0]['seq']) < int(b[0]['seq']))
    def require(ok, why):
        if not ok: errors.append(why)
    require(not any('_duplicate_fields' in r for r in rows + clients + uac), 'duplicate marker fields')
    activation = events('account-activated')
    proofs = events('login-proof-committed')
    registered = events('registered')
    minted = events('mint-committed')
    consumed = events('consume-committed')
    revoked = [r for r in events('transition-committed') if r.get('result') == '3']
    retired = events('service-retirement')
    reject = events('mint-syscall-rejected')
    inventory = events('inventory')
    # Authentication evidence must originate in role-checked kernel operations,
    # and name the same live Account incarnation as policy-accepted issuance.
    if not (activation and proofs and registered and uac and client('request-start') and client('policy-accepted-rendezvous')):
        missing.append('authenticated fixture or policy checkpoint missing')
    if activation and proofs:
        require(all(proofs[0].get(k) == activation[0].get(k) for k in ('pid','incarnation','vm')), 'login issuer identity differs from activated Account')
    if proofs: require(proofs[0].get('account') == '1', 'password proof issuer lacks trusted role')
    if registered:
        require(registered[0].get('sessiond') == '1' and int(registered[0].get('session','0')) > 0
                and int(registered[0].get('epoch','0')) > 0, 'registration identity invalid')
        for r in minted + consumed + revoked + retired + reject:
            require(all(r.get(k) == registered[0].get(k) for k in ('session','epoch')), 'transition uses a different session identity')
    expected_case, expected_stage = {'N1':(1,0),'N2':(2,1),'N3M':(3,0),'N3R':(9,0),'N4':(4,0),'N5':(5,1),'N6':(6,1),'N7R':(7,2),'N7C':(8,3),'N7OC':(10,2),'N7OR':(11,2)}[case]
    policy = [r for r in uac if r.get('case') == str(expected_case) and r.get('stage') == str(expected_stage)]
    if not policy: missing.append('required production policy boundary missing')
    if policy and activation and registered:
        require(policy[0].get('pid') == activation[0].get('pid') and all(policy[0].get(k) == registered[0].get(k) for k in ('session','epoch')), 'policy boundary issuer/session mismatch')
    cleaned = events('fixture-ipc-cleaned')
    require(bool(client('controller-ready')), 'controller identity missing')
    for checkpoint in ('fixture-complete', 'controller-ready'):
        for c in client(checkpoint):
            require(any(r.get('pid') == c.get('pid') and r.get('owned_endpoints') == '0'
                        and r.get('pending_call') == 'none' and r.get('reply_target') == 'none'
                        and r.get('deferred_targets') == '0'
                        and r.get('pending_commits') == '0' for r in cleaned), 'fixture references not retired')
    for r in rows:
        if 'cpu' in r: require(0 <= int(r['cpu']) < cpus, 'invalid CPU identity')
    sequences = [int(r['seq']) for r in rows if 'seq' in r]
    require(sequences == sorted(set(sequences)), 'duplicate or unordered authoritative sequence')
    require(bool(client('fixture-complete')), 'fixture did not complete')
    require(not any(m in raw for m in ('PANIC','panicked at','[FAULT]','checkpoint=FAIL')), 'guest failure')
    require(bool(inventory) and all(inventory[-1].get(k) == '0' for k in
                ('sessions','receipts','account_grants','pending_account_deliveries')), 'session subset not reconciled')
    require(bool(cleaned) and raw.rfind('checkpoint=inventory ') > raw.rfind('checkpoint=fixture-ipc-cleaned '),
            'terminal inventory after fixture cleanup missing')
    require(len(consumed) <= 1 and len(minted) <= 1, 'duplicate mint or consumption')
    require(not events('boundary-timeout'), 'controlled boundary timed out')
    for r in minted + consumed + reject:
        require(r.get('account') == '1', 'broker issuer lacks authenticated Account role')
        require(any(all(r.get(k) == a.get(k) for k in ('pid','incarnation','vm'))
                    and int(a['seq']) < int(r['seq']) for a in activation), 'unexpected broker service incarnation')
    requesters = events('authenticated-requester')
    bindings = events('request-binding')
    if not requesters or not bindings:
        missing.append('recipient incarnation/VM/call binding missing')
    for r in requesters:
        require(r.get('account') == r.get('sessiond') == r.get('tty') == '0', 'client acquired a trusted role')
        require(bool(client('request-start')) and r.get('pid') == client('request-start')[0].get('pid'), 'requester differs from real client')
    for b in bindings:
        require(any(all(b.get(a) == r.get(k) for a,k in
                       (('recipient','pid'),('recipient_inc','incarnation'),('recipient_vm','vm')))
                    for r in requesters), 'recipient binding differs from live incarnation')
        require(b.get('recipient_vm') == b.get('binding_vm') and int(b.get('request_generation','0')) > 0,
                'VM binding or IPC request generation invalid')
        require(any(all(b.get(a) == service.get(k) for a,k in
                        (('issuer','pid'),('issuer_inc','incarnation'),('issuer_vm','vm')))
                    and int(service['seq']) < int(b['seq']) for service in activation), 'binding issuer is not an activated Account incarnation')
    if inventory:
        require(inventory[0].get('sessions') == inventory[0].get('account_grants') == '0', 'initial inventory not observed before session registration')
    cross_cpu = 'NOT_FORCED'
    n4_prevented = False
    if case in ('N1','N2','N3M','N3R'):
        require(any(r.get('result') == str(0xC2FE if case in ('N1','N3R') else 0xC2FF)
                    for r in client('mint-response')), 'final mint outcome missing or wrong')
        if not revoked or not (minted or reject): missing.append('commit/revocation boundary missing')
        if case in ('N1','N3R'):
            require(not minted and ordered(revoked, reject), 'revoke-before-mint-rejection unproven')
        else:
            require(len(minted) == 1 and ordered(minted, revoked), 'mint-before-revocation unproven')
            require(bool(client('post-logout-authority-denied')), 'stale mint and retired consumption not checked')
        if case.startswith('N3'):
            ready = events('boundary-ready')
            grant = [r for r in ready if r.get('result') == '1']
            revoke = [r for r in ready if r.get('result') == '2']
            if not grant or not revoke or cpus < 2: missing.append('both native boundary entrants missing')
            else:
                require(grant[0].get('cpu') != revoke[0].get('cpu'), 'contenders did not run on separate CPUs')
                require(grant[0].get('account') == '1' and revoke[0].get('sessiond') == '1', 'contender role mismatch')
                first = minted or revoked
                require(ordered(grant, first) and ordered(revoke, first), 'both entrants not ready before first linearization')
                if not missing and not errors: cross_cpu = 'FORCED_AND_PASSED'
    elif case in ('N4','N5','N6'):
        closed = [r for r in events('peer-closed') if client('request-start') and r.get('recipient') == client('request-start')[0].get('pid')]
        reaped = events('service-ipc-retired')
        if not retired or len(activation) != 2 or not closed or not reaped or not reject:
            missing.append('retirement, pending call cancellation, replacement or old-session broker boundary missing')
        else:
            old, new = activation
            replacement_reject = [r for r in reject if all(r.get(k) == new.get(k) for k in ('pid','incarnation','vm'))]
            require(all(old.get(k) == retired[0].get('account_'+{'pid':'pid','incarnation':'inc','vm':'vm'}[k]) for k in ('pid','incarnation','vm')), 'retired Account differs from original issuer')
            require(all(old.get(k) != new.get(k) for k in ('incarnation','vm')), 'replacement identity is not distinct')
            require(retired[0].get('retiring_pid') == old.get('pid'), 'retirement was not caused by Account lifecycle')
            require(new.get('account') == '1', 'replacement lacks independent trusted activation')
            require(ordered(retired, closed) and ordered(closed, [new]) and ordered(reaped, [new]) and ordered([new], replacement_reject), 'lifecycle order unproven')
            require(bool(replacement_reject), 'old-session rejection not evaluated by replacement')
            require(bool(replacement_reject) and replacement_reject[0].get('result') == '11', 'old session denial is not broker SessionAuthorityUnavailable')
            require(bool(client('old-call-peer-closed')) and bool(client('replacement-old-session-denied')), 'recipient reconciliation missing')
            require(all(r.get('reply_target') == 'none' and r.get('deferred_targets') == '0'
                        and r.get('pending_call') == 'none' for r in reaped), 'old reply context survived retirement')
            require(all(reaped[0].get(k) == old.get(k) for k in ('pid','incarnation','vm')), 'retired IPC context belongs to a different service')
            require(any(r.get('recipient_inc') == q.get('incarnation') and r.get('recipient') == q.get('pid')
                        and int(r.get('request_generation','0')) > 0 for r in closed for q in requesters), 'cancelled call incarnation/generation mismatch')
        if case == 'N4':
            precommit = events('n4-precommit')
            stop = events('n4-stop-pending')
            held = events('n4-execution-held')
            signal = events('n4-manager-signal')
            if precommit:
                # This category proves deferred lifecycle, not the requested
                # row-retirement-before-commit ordering. The live-authority mint
                # is cancelled without delivery at actual endpoint retirement.
                require(cpus >= 2 and len(precommit) == len(stop) == len(held) == len(signal) == len(minted) == 1,
                        'N4 prevention transitions incomplete')
                for checkpoint in (precommit, stop, held, minted):
                    require(bool(activation) and all(all(r.get(k) == activation[0].get(k)
                            for k in ('pid','incarnation','vm'))
                            and all(r.get(k) == registered[0].get(k) for k in ('session','epoch'))
                            for r in checkpoint) if registered else False, 'N4 original issuer/session mismatch')
                require(ordered(precommit, signal) and ordered(signal, stop) and ordered(stop, held)
                        and ordered(held, minted) and ordered(minted, retired), 'N4 prevention order unproven')
                require(bool(stop) and stop[0].get('account') == '1' and stop[0].get('result') == '15',
                        'real pending SIGTERM missing')
                require(bool(held) and held[0].get('exiting') == '0'
                        and held[0].get('owning_core') == held[0].get('cpu'), 'old syscall execution ownership missing')
                require(bool(signal) and signal[0].get('signal') == '15'
                        and bool(precommit) and signal[0].get('cpu') != precommit[0].get('cpu'),
                        'actual concurrent manager signal missing')
                require(bool(activation) and all(all(s.get(a) == activation[0].get(k)
                        for a,k in (('issuer','pid'),('issuer_inc','incarnation'),('issuer_vm','vm')))
                        for s in signal), 'manager targeted wrong Account incarnation')
                require(bool(client('broker-pending-observed')), 'controller did not observe broker entry')
                require(any(r.get('pending_account_deliveries') == '1' for r in inventory), 'pre-retirement pending grant missing')
                require(any(all(b.get(k) == c.get(k) for k in ('recipient','recipient_inc','request_generation'))
                            and ordered(precommit, [b]) and ordered([b], signal)
                            for b in bindings for c in closed), 'precommit call not cancelled')
                require(not consumed, 'N4 pending authority was consumed')
                n4_prevented = True
            # Stage zero is userspace policy acceptance, not a broker entrant.
            # A replacement's denial must never certify a resumed old issuer.
            old_ready = [r for r in events('boundary-ready') if r.get('result') == '1'
                         and activation and all(r.get(k) == activation[0].get(k)
                                                for k in ('pid','incarnation','vm'))]
            old_reject = [r for r in reject if activation and all(r.get(k) == activation[0].get(k)
                                                                  for k in ('pid','incarnation','vm'))]
            if precommit:
                pass
            elif not old_ready or not old_reject:
                missing.append('original Account broker precommit entrant and post-retirement rejection missing; userspace cancellation only')
            else:
                require(ordered(old_ready, retired) and ordered(retired, old_reject), 'old issuer retirement/precommit order unproven')
        else:
            require(len(minted) == 1 and ordered(minted, retired), 'legitimate pending grant before retirement missing')
            require(any(r.get('pending_account_deliveries') == '1' for r in inventory), 'pending old Account reply not observed')
            require(any(all(b.get(a) == activation[0].get(k) for a,k in
                            (('issuer','pid'),('issuer_inc','incarnation'),('issuer_vm','vm')))
                        and b.get('recipient') == c.get('recipient')
                        and b.get('recipient_inc') == c.get('recipient_inc')
                        and b.get('request_generation') == c.get('request_generation')
                        and ordered([b], retired) for b in bindings for c in closed)
                    if len(activation) == 2 and retired else False,
                    'cancelled pending reply differs from original authorized call')
    else:
        rejected = events('consume-syscall-rejected')
        if not revoked or not events('consume-entry'): missing.append('consume/revocation boundary missing')
        require(len(minted) == 1, 'legitimate mint missing')
        if case in ('N7R','N7OR'): require(not consumed and ordered(revoked, rejected), 'logout-before-consume denial unproven')
        elif case == 'N7OC':
            require(len(consumed) == 1 and ordered(consumed, revoked) and ordered(consumed, rejected), 'overlapping consume-first order unproven')
        else:
            require(len(consumed) == 1 and ordered(consumed, rejected) and ordered(rejected, revoked), 'consume and immediate one-use denial before logout unproven')
        require(bool(client('second-consume-denied')), 'further consumption not rejected')
        if case in ('N7OC','N7OR'):
            ready = events('boundary-ready')
            consume_ready = [r for r in ready if r.get('result') == '4']
            revoke_ready = [r for r in ready if r.get('result') == '2']
            if cpus < 2 or not consume_ready or not revoke_ready:
                missing.append('overlapping consumption/revocation entrants missing')
            else:
                require(consume_ready[0].get('cpu') != revoke_ready[0].get('cpu'), 'consume/revoke contenders share CPU')
                first = consumed if case == 'N7OC' else revoked
                require(ordered(consume_ready, first) and ordered(revoke_ready, first), 'consume/revoke entrants not both admitted before linearization')
                if not missing and not errors: cross_cpu = 'FORCED_AND_PASSED'
    classification = 'NOT_FORCED' if missing else ('FORCED_AND_FAILED' if errors else 'FORCED_AND_PASSED')
    if n4_prevented and not errors and not missing:
        classification = 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT'
    return {'classification':classification, 'case':case, 'cpus':cpus, 'errors':errors,
            'missing_checkpoints':missing, 'markers':rows, 'client_markers':clients, 'uac_markers':uac,
            'scope':'real Account/sessiond boot-global broker; session subset only',
            'cross_cpu_contention':cross_cpu,
            'stale_reply_mechanism':'generation-qualified PeerClosed plus retired executable/reply context' if case == 'N5' else None,
            'actual_pid_reuse':'NOT_FORCED',
            'requested_retirement_before_commit_forced': False if n4_prevented else None,
            'n4_prevention_mechanism': 'nonpreemptible syscall retains execution ownership; pending SIGTERM is applied after the legitimate commit; endpoint retirement cancels its undelivered grant' if n4_prevented else None,
            'service_numeric_pid_reused':len(activation) == 2 and activation[0].get('pid') == activation[1].get('pid'),
            'resource_inventory':inventory[-1] if inventory else None}


def assess_replacement(raw, offset, cpus, *, identity=None, failure=None):
    """Views contain only recorded lines from one guest; never synthesized roles."""
    prefix, suffix = raw[:offset], raw[offset:]
    activations = [line for line in prefix.splitlines()
                   if line.startswith('[SESSION-E3] ') and 'checkpoint=account-activated ' in line]
    previous = assess_native(prefix, 'N6', cpus, identity=identity, failure=failure)
    view = (activations[-1] + '\n' if activations else '') + suffix
    fresh = assess_native(view, 'N7C', cpus, identity=identity, failure=failure)
    registrations = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines()
                     if line.startswith('[SESSION-E3] ') and 'checkpoint=registered ' in line]
    tuples = [tuple(r.get(k) for k in ('session','epoch')) for r in registrations]
    distinct = len(tuples) == 2 and tuples[0] != tuples[1]
    return {'classification':'FORCED_AND_PASSED' if distinct and all(
                r['classification'] == 'FORCED_AND_PASSED' for r in (previous, fresh)) else 'FORCED_AND_FAILED',
            'old_session':previous, 'replacement_session':fresh, 'session_tuples':tuples,
            'nonaliasing':distinct, 'view_character_offset':offset,
            'activation_line':activations[-1] if activations else None}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--section', choices=('build', 'normal', 'native', 'checks', 'compatibility'), required=True)
    parser.add_argument('--compat-section', choices=('production','authority','core','lifecycle','network'))
    parser.add_argument('--cpus', nargs='+', type=int, choices=(1, 2, 4, 12), default=[2])
    parser.add_argument('--cases', nargs='+', choices=CASES, default=['N1'])
    parser.add_argument('--image-manifest', type=Path)
    parser.add_argument('--timeout', type=int, default=120)
    parser.add_argument('--memory-mib', type=int, default=2048)
    parser.add_argument('--evidence-root', type=Path, default=BASE)
    parser.add_argument('--replacement-relogin', action='store_true',
                        help='After N6 use the real TTY login and one-use grant under replacement B')
    parser.add_argument('--n4-on-ap', action='store_true',
                        help='Build a separate N4-only test image with initial Account affinity on CPU 1')
    parser.add_argument('--h4', action='store_true', help='Prestarted authorized controller and independent completion protocol')
    parser.add_argument('--reaper-userspace', action='store_true',
                        help='N6 only: return the committed issuer to a bounded userspace loop before ordinary manager restart')
    parser.add_argument('--authenticated-reply', action='store_true',
                        help='N6 userspace-reaper mode with owner-ordered Account replies and real peer exec')
    args = parser.parse_args()
    if args.authenticated_reply and not args.reaper_userspace:
        parser.error('--authenticated-reply requires --reaper-userspace')
    if args.reaper_userspace and (not args.h4 or args.cases != ['N6']):
        parser.error('--reaper-userspace requires --h4 --cases N6')
    if any(c in ('H4R', 'S1', 'S2', 'S3') for c in args.cases) and not args.h4:
        parser.error('H4R requires --h4')
    if args.replacement_relogin and args.cases != ['N6']:
        parser.error('--replacement-relogin requires --cases N6')
    base = args.evidence_root.resolve()
    if args.section == 'compatibility':
        import sys
        import fastlane_f11b0e2_validate as replay
        if not args.compat_section: parser.error('--compat-section required')
        replay.BASE = base / 'compatibility'
        replay.BUILD = BUILD
        replay.ENV = ENV
        scratch = BUILD / 'tmp'
        scratch.mkdir(exist_ok=True)
        replay.tempfile.tempdir = str(scratch)
        sys.argv = [sys.argv[0], '--section', args.compat_section]
        replay.main()
        return
    out = base / (args.section + '-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    out.mkdir(parents=True)
    summary = {'phase': {'fastlane-f11b0e3s-evidence': 'F1.1B-0E3S', 'fastlane-f11b0e3r-evidence': 'F1.1B-0E3R', 'fastlane-f11b0e3t-evidence': 'F1.1B-0E3T'}.get(base.name, 'F1.1B-0E3'), 'source_sha256': sources(), 'checks': [], 'native': [], 'images': {},
               'gates': {'A': 'VERIFIED NATIVE', 'B': 'PARTIAL', 'session_authority': 'PARTIAL', 'F1.1B-0': 'BLOCKED', 'C': 'BLOCKED'}}
    if args.h4:
        summary['phase'] = 'F1.1B-0H4'
    summary['revision'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    uac_feature = 'session-reply-test' if args.authenticated_reply else ('session-reap-test' if args.reaper_userspace else ('session-h4-test' if args.h4 else 'session-race-test'))
    kernel_feature = 'authenticated_reply_test,key_inject' if args.authenticated_reply else ('native_session_h4_test,key_inject' if args.h4 else 'native_session_race_test,key_inject')
    def save():
        (out / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    def check(name, cmd, extra=None, timeout=1200):
        env = {**ENV, **(extra or {})}
        start = time.monotonic()
        with (out / (name + '.log')).open('w') as log:
            log.write(json.dumps({'command': cmd, 'environment': env}) + '\n'); log.flush()
            try:
                code = subprocess.run(cmd, cwd=ROOT, env={**os.environ, **env}, stdout=log,
                                      stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        summary['checks'].append({'name': name, 'command': cmd, 'environment': env, 'exit_code': code,
                                  'classification': 'PASS' if code == 0 else 'FAIL', 'seconds': time.monotonic()-start,
                                  'log': str((out / (name+'.log')).relative_to(ROOT))})
        print(name, summary['checks'][-1]['classification'], flush=True); save()
        return code == 0
    try:
        if args.section in ('build', 'normal'):
            test = args.section == 'build'
            for package in ('sunlight-init', 'sunlightd', 'sunlight-tty-server', 'sunlight-uac',
                            'sunlight-sessiond', 'sunlight-vfs-server', 'sunlightctl'):
                # TTY enables libc's allocator; other services own theirs.
                # Match tools/build.sh's separate package invocations so Cargo
                # feature unification cannot install two global allocators.
                service_cmd = ['cargo', 'build', '--locked', '--offline', '--release', '-p', package]
                if test and package == 'sunlight-uac': service_cmd += ['--features', uac_feature]
                if test and args.h4 and package in ('sunlightctl', 'sunlightd'): service_cmd += ['--features', 'session-h4-test']
                if not check('service-'+package, service_cmd, {'RUSTFLAGS': SERVICE_FLAGS}): raise SystemExit(1)
            for name in ('sunlight-init', 'sunlightd', 'sunlight-tty-server', 'uac_service',
                         'sunlight-sessiond', 'sunlight-vfs-server', 'sunlightctl') + (('session-race-probe',) if test else ()):
                shutil.copy2(BUILD / 'x86_64-unknown-none/release' / name, ROOT / 'target/x86_64-unknown-none/release' / name)
            kernel_cmd = ['cargo', 'build', '--locked', '--offline', '-p', 'sunlight-kernel', '--bin', 'sunlight-kernel']
            if test: kernel_cmd += ['--features', kernel_feature]
            test_environment = {'SUNLIGHT_INJECT_PHASE': 'desktop_login',
                                'SUNLIGHT_SESSION_N4_AP': '1' if args.n4_on_ap or args.h4 else '0'} if test else {}
            if not check('kernel', kernel_cmd, test_environment): raise SystemExit(1)
            built = BUILD / 'x86_64-unknown-none/debug/sunlight-kernel'
            elf, iso = out / 'kernel.elf', out / 'fixture.iso'
            if not check('strip', ['objcopy', '--strip-debug', str(built), str(elf)]): raise SystemExit(1)
            scratch = out / 'iso-build'; scratch.mkdir()
            shutil.copy2(ROOT / 'limine.conf', scratch / 'limine.conf')
            if not check('iso', ['bash', 'tools/make_hybrid_iso.sh', str(elf), str(iso), str(ROOT/'target/limine'), str(scratch)]): raise SystemExit(1)
            payloads = {}
            raw = built.read_bytes()
            embedded = out / 'embedded'; embedded.mkdir()
            for name in ('sunlight-init', 'sunlightd', 'sunlight-tty-server', 'uac_service', 'sunlight-sessiond', 'sunlight-vfs-server', 'sunlightctl') + (('session-race-probe',) if test else ()):
                p = ROOT / 'target/x86_64-unknown-none/release' / name
                data = p.read_bytes(); offset = raw.find(data)
                if offset < 0: raise RuntimeError('current binary not embedded: ' + name)
                shutil.copy2(p, embedded / name)
                payloads[name] = {'sha256': digest(p), 'bytes': len(data), 'embedded_offset': offset}
            if not test and any(marker in raw for marker in (b'[SESSION-E3]', b'[SESSION-E3-UAC]', b'[SESSION-E3-CLIENT]', b'[H4-CTL]', b'[H4-MANAGER]', b'[H3-TRACE]', b'[REAP-TEST]', b'[AUTH-REPLY]', b'[AUTH-KERNEL]', b'[AUTH-TERMINAL]', b'[AUTH-BINDING]', b'[AUTH-OBSERVE]')):
                raise RuntimeError('test instrumentation present in ordinary image')
            summary['images']['fixture' if test else 'production'] = {'kernel_sha256': digest(built), 'elf_sha256': digest(elf),
                'iso_sha256': digest(iso), 'iso': str(iso), 'features': kernel_feature.split(',') if test else [],
                'test_environment': test_environment,
                'service_features': ({'sunlight-uac': [uac_feature], 'sunlightd': ['session-h4-test'],
                                      'sunlightctl': ['session-h4-test']} if test and args.h4 else {}),
                'embedded_services': payloads, 'source_sha256': summary['source_sha256']}
        elif args.section == 'checks':
            host = ['cargo', 'test', '--locked', '--offline', '--target', 'x86_64-unknown-linux-gnu']
            check('kernel-host', host + ['-p', 'sunlight-kernel', '--lib', '--', '--test-threads=1'])
            packages = ['sunlight-ipc', 'sunlight-uac', 'sunlight-sessiond', 'sunlight-fs', 'sunlightd', 'sunlight-audiod', 'sunlight-elf', 'sunlight-fastlane-ring', 'sunlight-fastlane-doorbell']
            check('services-host', host + sum((['-p', p] for p in packages), []) + ['--lib'])
            check('sunlightd-runtime', host + ['-p', 'sunlightd', '--bin', 'sunlightd', '--features', 'dependency-test'])
            check('scheduler-host', ['python3', 'tools/test_scheduler_host.py'])
            check('parsers', ['python3', '-m', 'unittest', 'discover', '-s', 'tools', '-p', 'test_fastlane_f11b0*.py'])
            check('strict-clippy', ['cargo', 'clippy', '--locked', '--offline', '--target', 'x86_64-unknown-linux-gnu', '-p', 'sunlight-kernel', '--lib', '--', '-D', 'warnings'])
            check('media-host', host + ['-p', 'sunlight-media', '--lib'])
            check('workspace-format', ['cargo', 'fmt', '--all', '--', '--check'])
            check('diff-check', ['git', 'diff', '--check'])
            check('miri', ['cargo', 'miri', '--version'])
        else:
            if args.image_manifest is None: parser.error('--image-manifest required for native')
            manifest = json.loads(args.image_manifest.read_text())
            image = manifest['images']['fixture']; iso = Path(image['iso'])
            uac_features = image.get('service_features', {}).get('sunlight-uac', [])
            reaper_image = any(f in uac_features for f in ('session-reap-test', 'session-reply-test'))
            if reaper_image != args.reaper_userspace:
                parser.error('image userspace reaper mode differs from requested fixture')
            if ('session-reply-test' in uac_features) != args.authenticated_reply:
                parser.error('image authenticated reply mode differs from requested fixture')
            if image.get('test_environment', {}).get('SUNLIGHT_SESSION_N4_AP') == '1' and args.cases != ['N4'] and not args.h4:
                parser.error('N4 AP-placement image supports only --cases N4; use ordinary fixture for other races')
            if digest(iso) != image['iso_sha256']: raise RuntimeError('ISO hash mismatch')
            elf = args.image_manifest.parent / 'kernel.elf'
            if digest(elf) != image['elf_sha256']: raise RuntimeError('ELF hash mismatch')
            def services_match():
                return all(digest(args.image_manifest.parent / 'embedded' / name) == data['sha256']
                           for name, data in image['embedded_services'].items())
            if not services_match(): raise RuntimeError('embedded executable archive hash mismatch')
            summary['images']['fixture'] = image
            summary['image_manifest'] = str(args.image_manifest)
            if summary['source_sha256'] != image['source_sha256']:
                raise RuntimeError('source snapshot differs from image build')
            for cpu in args.cpus:
                for case in args.cases:
                    if cpu == 1 and (case.startswith('N3') or case in ('N7OC','N7OR')):
                        summary['native'].append({'case':case, 'cpus':cpu, 'classification':'NOT_FORCED', 'reason':'cross-CPU case requires at least two CPUs'})
                        save()
                        continue
                    name = f'{case}-{cpu}cpu'
                    serial = out / (name+'.serial.log'); serial.touch()
                    sock = out / (name+'.qmp.sock')
                    # Linux UNIX socket paths must stay below 108 bytes.
                    sock = Path('/tmp') / ('e3-' + out.name[-14:] + '-' + name + '.sock')
                    cmd = ['qemu-system-x86_64', '-accel', 'tcg,thread=multi', '-cpu', 'max', '-smp', str(cpu), '-m', str(args.memory_mib), '-cdrom', str(iso), '-serial', 'file:'+str(serial), '-display', 'none', '-nic', 'none', '-no-reboot', '-no-shutdown', '-device', 'virtio-rng-pci,disable-modern=on', '-qmp', 'unix:'+str(sock)+',server=on,wait=off']
                    start = time.monotonic(); failure = None; qmp = None; replacement_offset = None
                    fixture_id = str(secrets.randbelow(900000000) + 100000000) if args.h4 else None
                    def raw(): return serial.read_text(errors='replace')
                    def await_marker(marker, count=1):
                        while time.monotonic()-start < args.timeout and proc.poll() is None:
                            data = raw()
                            if data.count(marker) >= count: return
                            if any(m in data for m in ('PANIC', 'panicked at', 'checkpoint=FAIL')): raise RuntimeError('guest failure')
                            time.sleep(.1)
                        if proc.poll() is not None:
                            raise RuntimeError('QEMU exited before checkpoint: ' + marker)
                        raise TimeoutError('missing checkpoint: ' + marker)
                    def key(*codes):
                        for down, ordered in ((True, codes), (False, tuple(reversed(codes)))):
                            for code in ordered:
                                qmp.call('input-send-event', {'events': [{'type':'key', 'data': {'down':down, 'key': {'type':'qcode', 'data':code}}}]})
                            time.sleep(.08)
                    def type_text(value):
                        for char in value:
                            if char.isupper(): key('shift', char.lower())
                            else: key({' ':'spc', '/':'slash', '-':'minus', '\n':'ret', '\t':'tab'}.get(char, char))
                    def await_case(selected):
                        if args.h4:
                            from fastlane_f11b0h4_validate import guest_terminal_errors
                            while time.monotonic()-start < args.timeout and proc.poll() is None:
                                errors = guest_terminal_errors(raw(), selected, fixture_id)
                                if not errors: return
                                if any(m in raw() for m in ('PANIC', 'panicked at', 'checkpoint=FAIL')):
                                    raise RuntimeError('guest assertion failed')
                                time.sleep(.1)
                            raise TimeoutError('H4 terminal protocol: ' + '; '.join(errors))
                        await_marker('case='+selected+' checkpoint=fixture-complete')
                        await_marker('exit: /tmp/session-race-probe '+selected+' -> 0')
                        markers = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw().splitlines()
                                   if line.startswith('[SESSION-E3-CLIENT] case='+selected+' ')]
                        for marker in markers:
                            if marker.get('checkpoint') in ('fixture-complete','controller-ready'):
                                await_marker("[SCHED] process_reaped pid="+marker['pid']+" name='")
                    with (out / (name+'.qemu.log')).open('w') as log:
                        proc = subprocess.Popen(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
                        try:
                            await_marker('[SESSION] switched to F2 GraphicalDesktop')
                            qmp = Qmp(str(sock)); key('ctrl', 'f1')
                            await_marker('[SESSION] switched to F1 TTY/Login')
                            # The existing login keeps the dropdown focused and
                            # retains its password field. Reverify via TTY mode.
                            key('spc'); key('ret')
                            await_marker('[TTY]  Built-in shell ready')
                            await_marker("sunshell registered as 'sshl'")
                            for text in ('touch /tmp/session-race-probe\n', '/tmp/session-race-probe '+case+(' '+fixture_id if args.h4 else '')+'\n'):
                                type_text(text)
                                time.sleep(.5)
                            await_case(case)
                            if args.replacement_relogin:
                                replacement_offset = len(raw())
                                type_text('exit\n'); time.sleep(.5)
                                key('ret'); type_text('root'); key('tab'); key('spc'); key('ret')
                                await_marker('checkpoint=registered ', 2)
                                await_marker('[SESSION] switched to F2 GraphicalDesktop', 2)
                                key('ctrl', 'f1')
                                await_marker('[SESSION] switched to F1 TTY/Login', 2)
                                key('spc'); key('ret')
                                await_marker('[TTY]  Built-in shell ready', 2)
                                type_text('/tmp/session-race-probe N7C\n'); await_case('N7C')
                        except Exception as error:
                            failure = {'type': type(error).__name__, 'message': str(error)}
                        finally:
                            if proc.poll() is None:
                                proc.terminate()
                                try: proc.wait(timeout=5)
                                except subprocess.TimeoutExpired: proc.kill(); proc.wait()
                            if qmp is not None: qmp.stream.close(); qmp.socket.close()
                    identity = {'iso_matches': digest(iso) == image['iso_sha256'],
                                'elf_matches': digest(elf) == image['elf_sha256'],
                                'services_match': services_match(),
                                'source_matches': sources() == image['source_sha256'],
                                'iso_sha256': image['iso_sha256'], 'elf_sha256': image['elf_sha256']}
                    if args.h4:
                        from fastlane_f11b0h4_validate import assess, host_ack_errors
                        detail = assess(raw(), case, cpu, fixture_id, identity=identity, failure=failure)
                        if args.reaper_userspace:
                            from fastlane_f11b0h4_validate import reaper_userspace_errors
                            mode_errors = reaper_userspace_errors(raw(), fixture_id)
                            detail['userspace_reaper_errors'] = mode_errors
                            detail['userspace_reaper'] = 'PASS' if not mode_errors else 'FAIL_CURRENT'
                            if mode_errors:
                                detail['errors'].extend(mode_errors)
                                detail['classification'] = 'FORCED_AND_FAILED'
                                detail['terminal_protocol'] = 'BLOCKED'
                        if args.authenticated_reply:
                            from fastlane_authenticated_reply_validate import assess as assess_replies
                            replies = assess_replies(raw(), cpu)
                            detail['authenticated_replies'] = replies
                            if replies['status'] != 'PASS':
                                detail['errors'].extend(replies['errors'])
                                detail['classification'] = 'FORCED_AND_FAILED'
                                detail['terminal_protocol'] = 'BLOCKED'
                        if detail['terminal_protocol'] == 'GUEST_VALIDATED_PENDING_HOST_ACK':
                            ack = {'fixture': fixture_id, 'case': case, 'cpus': cpu, 'serial_sha256': digest(serial),
                                   'iso_sha256': digest(iso), 'elf_sha256': digest(elf),
                                   'security_assertions': 'PASS', 'cleanup': 'PASS', 'identity': identity}
                            ack_path = out / (name+'.host-ack.json')
                            ack_path.write_text(json.dumps(ack, indent=2)+'\n')
                            ack_errors = host_ack_errors(json.loads(ack_path.read_text()), fixture=fixture_id,
                                case=case, cpus=cpu, serial_sha256=digest(serial), iso_sha256=digest(iso), elf_sha256=digest(elf))
                            if ack_errors:
                                raise RuntimeError('; '.join(ack_errors))
                            detail['terminal_protocol'] = 'PASS'
                            detail['host_terminal_acknowledged'] = True
                            detail['host_ack'] = str(ack_path.relative_to(ROOT))
                    else:
                        detail = (assess_replacement(raw(), replacement_offset, cpu, identity=identity, failure=failure)
                              if args.replacement_relogin and replacement_offset is not None
                              else assess_native(raw(), case, cpu, identity=identity, failure=failure))
                    if args.replacement_relogin and replacement_offset is None:
                        detail['classification'] = 'NOT_FORCED'
                        detail['missing_checkpoints'].append('replacement login not attempted')
                    summary['native'].append({'name':name, 'case':case, 'cpus':cpu, 'command':cmd,
                        'exit_code':proc.returncode, 'seconds':time.monotonic()-start, 'failure':failure,
                        'timeout_classification':'MISSING_CHECKPOINT' if failure and failure['type']=='TimeoutError' else None,
                        'serial':str(serial.relative_to(ROOT)), 'serial_sha256':digest(serial),
                        'iso_sha256':digest(iso), 'validator':detail})
                    print(name, detail['classification'], failure or '', flush=True); save()
                    if detail['classification'] not in ACCEPTED:
                        raise RuntimeError('native case failed; stop matrix expansion and inspect preserved attempt')
    finally:
        summary['source_sha256_after'] = sources()
        summary['source_hashes_unchanged'] = summary['source_sha256'] == summary['source_sha256_after']
        save(); print('Evidence:', out.relative_to(ROOT), flush=True)
    if not summary['source_hashes_unchanged'] or any(c['exit_code'] for c in summary['checks']) or any(r.get('validator',{}).get('classification') not in (None, *ACCEPTED) for r in summary['native']):
        raise SystemExit(1)


if __name__ == '__main__': main()
